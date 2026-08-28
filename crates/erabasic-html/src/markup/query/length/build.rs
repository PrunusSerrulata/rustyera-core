use super::super::super::{HtmlAttribute, HtmlElementKind, HtmlElementSemantic, HtmlNode};
use super::super::{HtmlMappedText, HtmlScalarBoundary};
use super::{
    Button, Entry, HtmlAlignment, HtmlDocument, HtmlLengthCut, HtmlLengthProbe,
    HtmlLengthProbeKind, HtmlMappedDocument, HtmlQueryError, HtmlQueryErrorKind, HtmlQueryLimits,
    HtmlSourceRange, HtmlStringLengthSettings, Layout, Part, PartKind, geometry, input_error,
    invalid_measurement, resource_limit,
};
use std::collections::BTreeMap;

pub(super) struct Built {
    pub probes: Vec<HtmlLengthProbe>,
    pub parts: Vec<Part>,
    pub layouts: Vec<Layout>,
    pub root_layout: usize,
    pub work_bytes: usize,
    pub measurement_units: usize,
}

#[derive(Clone, Copy, Default)]
struct Style<'a> {
    flags: u8,
    face: Option<&'a str>,
    color: Option<u32>,
    button_color: Option<u32>,
    font_depth: usize,
}

#[derive(Clone, Copy)]
struct ButtonStyle {
    clickable: bool,
    position: Option<i32>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ParagraphState {
    Unseen,
    Open,
    Closed,
}

struct State {
    layout: Layout,
    pending: Vec<usize>,
    button: Option<ButtonStyle>,
    clear_button: bool,
    line_head: bool,
    paragraph: ParagraphState,
    nobr_closed: bool,
}

impl State {
    fn new(width: i64, clear_button: bool) -> Self {
        Self {
            layout: Layout {
                entries: Vec::new(),
                no_break: false,
                alignment: HtmlAlignment::Left,
                width,
            },
            pending: Vec::new(),
            button: None,
            clear_button,
            line_head: true,
            paragraph: ParagraphState::Unseen,
            nobr_closed: false,
        }
    }

    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        self.layout.entries.push(Entry::Button(Button {
            parts: std::mem::take(&mut self.pending),
            clickable: self.button.is_some_and(|button| button.clickable),
            position: self.button.and_then(|button| button.position),
        }));
    }

    fn line_break(&mut self) {
        self.flush();
        self.layout.entries.push(Entry::Break);
    }
}

struct Builder<'a> {
    source: Option<&'a str>,
    texts: BTreeMap<Vec<usize>, &'a HtmlMappedText>,
    settings: HtmlStringLengthSettings,
    limits: HtmlQueryLimits,
    work: usize,
    measurements: usize,
    built: Built,
}

pub(super) fn build(
    source: &str,
    mapped: &HtmlMappedDocument,
    settings: HtmlStringLengthSettings,
    limits: HtmlQueryLimits,
) -> Result<Built, HtmlQueryError> {
    build_inner(
        Some(source),
        &mapped.document,
        &mapped.texts,
        settings,
        limits,
    )
}

pub(super) fn build_document(
    document: &HtmlDocument,
    settings: HtmlStringLengthSettings,
    limits: HtmlQueryLimits,
) -> Result<Built, HtmlQueryError> {
    build_inner(None, document, &[], settings, limits)
}

fn build_inner(
    source: Option<&str>,
    document: &HtmlDocument,
    texts: &[HtmlMappedText],
    settings: HtmlStringLengthSettings,
    limits: HtmlQueryLimits,
) -> Result<Built, HtmlQueryError> {
    let mut builder = Builder {
        source,
        texts: texts
            .iter()
            .map(|text| (text.node_path.clone(), text))
            .collect(),
        settings,
        limits,
        work: source.map_or(0, str::len),
        measurements: 0,
        built: Built {
            probes: Vec::new(),
            parts: Vec::new(),
            layouts: Vec::new(),
            root_layout: 0,
            work_bytes: 0,
            measurement_units: 0,
        },
    };
    let mut state = State::new(i64::from(settings.drawable_width_pixels), false);
    builder.nodes(
        &document.nodes,
        &mut Vec::new(),
        Style::default(),
        &mut state,
    )?;
    builder.built.root_layout = builder.finish_layout(state)?;
    builder.built.work_bytes = builder.work;
    builder.built.measurement_units = builder.measurements;
    Ok(builder.built)
}

impl<'a> Builder<'a> {
    fn reserve(&mut self, bytes: usize, measurements: usize) -> Result<(), HtmlQueryError> {
        self.work = self.work.saturating_add(bytes);
        self.measurements = self.measurements.saturating_add(measurements);
        if self.work > self.limits.maximum_work_bytes
            || self.measurements > self.limits.maximum_measurements
            || self.built.parts.len() >= self.limits.maximum_nodes
        {
            return Err(resource_limit());
        }
        Ok(())
    }

    fn finish_layout(&mut self, mut state: State) -> Result<usize, HtmlQueryError> {
        state.flush();
        if state.layout.entries.len() > self.limits.maximum_nodes
            || self.built.layouts.len() >= self.limits.maximum_nodes
        {
            return Err(resource_limit());
        }
        for entry in &state.layout.entries {
            if let Entry::Button(button) = entry
                && button.position.is_some()
                && (!state.layout.no_break || state.layout.alignment != HtmlAlignment::Left)
            {
                return Err(input_error(
                    HtmlQueryErrorKind::InvalidMarkup,
                    "HTML pos requires nobr and left alignment",
                ));
            }
        }
        let index = self.built.layouts.len();
        self.built.layouts.push(state.layout);
        Ok(index)
    }

    fn nodes(
        &mut self,
        nodes: &'a [HtmlNode],
        path: &mut Vec<usize>,
        style: Style<'a>,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        if path.len() > self.limits.maximum_depth {
            return Err(resource_limit());
        }
        for (index, node) in nodes.iter().enumerate() {
            path.push(index);
            match node {
                HtmlNode::Text { text, start, end } => {
                    self.text_node(text, *start, *end, path, style, state)?;
                }
                HtmlNode::Element { .. } => self.element(node, path, style, state)?,
            }
            path.pop();
        }
        Ok(())
    }

    fn text_node(
        &mut self,
        text: &str,
        source_start: u64,
        source_end: u64,
        path: &[usize],
        style: Style<'_>,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        let generated;
        let mapping = if let Some(mapping) = self.texts.get(path) {
            *mapping
        } else {
            if self.source.is_some() {
                return Err(invalid_measurement());
            }
            let scalars = text.chars().count();
            if text.len() > self.limits.maximum_source_bytes
                || scalars > self.limits.maximum_scalars
            {
                return Err(resource_limit());
            }
            self.reserve(
                scalars
                    .saturating_add(1)
                    .saturating_mul(std::mem::size_of::<HtmlScalarBoundary>())
                    .saturating_add(path.len().saturating_mul(std::mem::size_of::<usize>())),
                0,
            )?;
            generated = canonical_mapping(text, path, source_range(source_start, source_end)?);
            &generated
        };
        let mut start = 0;
        for (index, boundaries) in mapping.boundaries.windows(2).enumerate() {
            // Only literal source newlines are FlagBr. An entity which
            // decodes to LF remains inside the measured styled part.
            let newline = if let Some(source) = self.source {
                source.get(boundaries[0].source_byte..boundaries[1].source_byte) == Some("\n")
            } else {
                text.get(boundaries[0].decoded_utf8..boundaries[1].decoded_utf8) == Some("\n")
            };
            if newline {
                self.text_part(text, mapping, start, index, style, state)?;
                state.line_break();
                start = index + 1;
            }
        }
        self.text_part(
            text,
            mapping,
            start,
            mapping.boundaries.len() - 1,
            style,
            state,
        )
    }

    fn element(
        &mut self,
        node: &'a HtmlNode,
        path: &mut Vec<usize>,
        style: Style<'a>,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        let HtmlNode::Element {
            kind,
            semantic,
            children,
            start,
            end,
            ..
        } = node
        else {
            return Err(invalid_measurement());
        };
        let range = source_range(*start, *end)?;
        match semantic {
            HtmlElementSemantic::Style | HtmlElementSemantic::Font { .. } => {
                self.nodes(children, path, inline_style(*kind, semantic, style)?, state)?;
            }
            HtmlElementSemantic::Paragraph { alignment } => {
                self.paragraph(children, path, style, *alignment, state)?;
            }
            HtmlElementSemantic::NoBreak => {
                if !state.line_head || state.layout.no_break {
                    return Err(input_error(
                        HtmlQueryErrorKind::InvalidMarkup,
                        "nobr is not at the initial line head",
                    ));
                }
                state.layout.no_break = true;
                self.nodes(children, path, style, state)?;
                state.nobr_closed = true;
            }
            HtmlElementSemantic::Button {
                value, position, ..
            } => {
                let button = ButtonStyle {
                    clickable: value.is_some() && !state.clear_button,
                    position: *position,
                };
                self.button_children(children, path, style, button, state)?;
            }
            HtmlElementSemantic::NonButton { position, .. } => {
                let button = ButtonStyle {
                    clickable: false,
                    position: *position,
                };
                self.button_children(children, path, style, button, state)?;
            }
            HtmlElementSemantic::ClearButton { .. } => {
                let previous = state.clear_button;
                state.clear_button = true;
                self.nodes(children, path, style, state)?;
                state.clear_button = previous;
            }
            HtmlElementSemantic::Break => state.line_break(),
            HtmlElementSemantic::Division { .. } => {
                self.division(node, range, path, style, state)?;
            }
            HtmlElementSemantic::Shape { .. } => self.shape(node, range, state)?,
            HtmlElementSemantic::Image { .. } => self.image(node, range, state)?,
        }
        Ok(())
    }

    fn paragraph(
        &mut self,
        children: &'a [HtmlNode],
        path: &mut Vec<usize>,
        style: Style<'a>,
        alignment: HtmlAlignment,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        if !state.line_head || state.layout.no_break || state.paragraph != ParagraphState::Unseen {
            return Err(input_error(
                HtmlQueryErrorKind::InvalidMarkup,
                "p is not at the initial line head",
            ));
        }
        state.paragraph = ParagraphState::Open;
        state.layout.alignment = alignment;
        self.nodes(children, path, style, state)?;
        state.paragraph = ParagraphState::Closed;
        Ok(())
    }

    fn button_children(
        &mut self,
        children: &'a [HtmlNode],
        path: &mut Vec<usize>,
        style: Style<'a>,
        button: ButtonStyle,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        state.flush();
        state.button = Some(button);
        self.nodes(children, path, style, state)?;
        state.flush();
        state.button = None;
        Ok(())
    }

    fn division(
        &mut self,
        node: &'a HtmlNode,
        range: HtmlSourceRange,
        path: &mut Vec<usize>,
        style: Style<'_>,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        let HtmlNode::Element {
            semantic:
                HtmlElementSemantic::Division {
                    width,
                    box_model,
                    relative,
                    ..
                },
            children,
            ..
        } = node
        else {
            return Err(invalid_measurement());
        };
        if state.button.is_some() || style.flags != 0 || style.font_depth != 0 {
            return Err(input_error(
                HtmlQueryErrorKind::InvalidMarkup,
                "division begins inside an unclosed button/font/style",
            ));
        }
        let mut width = geometry::integer_length(*width, self.settings.font_size_pixels)?;
        for edges in [&box_model.margin, &box_model.border, &box_model.padding]
            .into_iter()
            .flatten()
        {
            for side in [1, 3] {
                width = geometry::add_pixels(
                    width,
                    -geometry::integer_length(edges[side], self.settings.font_size_pixels)?,
                )?;
            }
        }
        let mut child = State::new(
            if width > 0 {
                width
            } else {
                i64::from(self.settings.drawable_width_pixels)
            },
            state.clear_button,
        );
        self.nodes(children, path, Style::default(), &mut child)?;
        self.finish_layout(child)?;
        self.atomic(
            node,
            range,
            HtmlLengthProbeKind::FixedSlot,
            PartKind::Division {
                absolute: !relative,
            },
            state,
        )
    }

    fn shape(
        &mut self,
        node: &HtmlNode,
        range: HtmlSourceRange,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        let HtmlNode::Element {
            semantic:
                HtmlElementSemantic::Shape {
                    kind,
                    parameters,
                    color,
                    button_color,
                },
            ..
        } = node
        else {
            return Err(invalid_measurement());
        };
        if let Some(advance) =
            geometry::shape_advance(kind, parameters, self.settings.font_size_pixels)?
        {
            self.atomic(
                node,
                range,
                HtmlLengthProbeKind::FixedSlot,
                PartKind::Shape { advance },
                state,
            )?;
        } else {
            self.reserve(
                range
                    .end
                    .saturating_sub(range.start)
                    .saturating_mul(4)
                    .saturating_add(4096),
                1,
            )?;
            let fallback =
                geometry::shape_fallback(kind, parameters, *color, *button_color, self.settings);
            let utf16_length = fallback.encode_utf16().count();
            let document = text_document(fallback, Style::default(), range).0;
            self.push_probe(
                document,
                range,
                HtmlLengthProbeKind::FallbackText,
                PartKind::Fallback { utf16_length },
                state,
            );
        }
        Ok(())
    }

    fn image(
        &mut self,
        node: &HtmlNode,
        range: HtmlSourceRange,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        let HtmlNode::Element {
            semantic:
                HtmlElementSemantic::Image {
                    source,
                    hover_source,
                    mask_source,
                    height,
                    width,
                    y,
                },
            ..
        } = node
        else {
            return Err(invalid_measurement());
        };
        self.reserve(
            range
                .end
                .saturating_sub(range.start)
                .saturating_mul(8)
                .saturating_add(4096),
            1,
        )?;
        let fallback = geometry::image_fallback(
            source,
            hover_source.as_deref(),
            mask_source.as_deref(),
            *height,
            *width,
            *y,
            self.settings.font_size_pixels,
        )?;
        let fallback_utf16_length = fallback.encode_utf16().count();
        let missing_document = text_document(fallback, Style::default(), range).0;
        self.push_probe(
            HtmlDocument {
                nodes: vec![shallow_node(node)],
            },
            range,
            HtmlLengthProbeKind::ImageSlot { missing_document },
            PartKind::Image {
                height: *height,
                width: *width,
                fallback_utf16_length,
            },
            state,
        );
        Ok(())
    }

    fn text_part(
        &mut self,
        text: &str,
        mapping: &HtmlMappedText,
        start: usize,
        end: usize,
        style: Style<'_>,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        if start == end {
            return Ok(());
        }
        state.line_head = false;
        let begin = mapping.boundaries[start];
        let finish = mapping.boundaries[end];
        if self
            .source
            .is_some_and(|source| finish.source_byte == source.len())
            && (state.paragraph == ParagraphState::Closed || state.nobr_closed)
        {
            return Err(input_error(
                HtmlQueryErrorKind::InvalidMarkup,
                "text follows a closed p/nobr scope",
            ));
        }
        self.reserve(
            (finish.decoded_utf8 - begin.decoded_utf8)
                .saturating_mul(4)
                .saturating_add(style.face.map_or(0, str::len).saturating_mul(4))
                .saturating_add((end - start + 1).saturating_mul(
                    std::mem::size_of::<HtmlLengthCut>() + std::mem::size_of::<i64>(),
                ))
                .saturating_add(4096),
            end - start + 2,
        )?;
        let source = if self.source.is_some() {
            HtmlSourceRange {
                start: begin.source_byte,
                end: finish.source_byte,
            }
        } else {
            mapping.range
        };
        let (document, text_node_path) = text_document(
            text[begin.decoded_utf8..finish.decoded_utf8].into(),
            style,
            source,
        );
        let cuts = mapping.boundaries[start..=end]
            .iter()
            .map(|boundary| HtmlLengthCut {
                decoded_utf8: boundary.decoded_utf8 - begin.decoded_utf8,
                decoded_utf16: boundary.decoded_utf16 - begin.decoded_utf16,
                source_byte: self.source.map(|_| boundary.source_byte),
            })
            .collect();
        self.push_probe(
            document,
            source,
            HtmlLengthProbeKind::TextPart {
                text_node_path,
                cuts,
            },
            PartKind::Text,
            state,
        );
        Ok(())
    }

    fn atomic(
        &mut self,
        node: &HtmlNode,
        range: HtmlSourceRange,
        probe: HtmlLengthProbeKind,
        part: PartKind,
        state: &mut State,
    ) -> Result<(), HtmlQueryError> {
        self.reserve(
            range
                .end
                .saturating_sub(range.start)
                .saturating_mul(4)
                .saturating_add(4096),
            1,
        )?;
        self.push_probe(
            HtmlDocument {
                nodes: vec![shallow_node(node)],
            },
            range,
            probe,
            part,
            state,
        );
        Ok(())
    }

    fn push_probe(
        &mut self,
        document: HtmlDocument,
        source: HtmlSourceRange,
        kind: HtmlLengthProbeKind,
        part: PartKind,
        state: &mut State,
    ) {
        let probe = self.built.probes.len();
        self.built.probes.push(HtmlLengthProbe {
            id: probe as u64,
            document,
            kind,
            source,
        });
        state.pending.push(self.built.parts.len());
        self.built.parts.push(Part { probe, kind: part });
    }
}

fn source_range(start: u64, end: u64) -> Result<HtmlSourceRange, HtmlQueryError> {
    Ok(HtmlSourceRange {
        start: usize::try_from(start).map_err(|_| invalid_measurement())?,
        end: usize::try_from(end).map_err(|_| invalid_measurement())?,
    })
}

fn inline_style<'a>(
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
        } => Ok(Style {
            face: face.as_deref().or(style.face),
            color: color.or(style.color),
            button_color: button_color.or(style.button_color),
            font_depth: style.font_depth + 1,
            ..style
        }),
        _ => Err(invalid_measurement()),
    }
}

fn canonical_mapping(text: &str, path: &[usize], range: HtmlSourceRange) -> HtmlMappedText {
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

fn shallow_node(node: &HtmlNode) -> HtmlNode {
    match node {
        HtmlNode::Element {
            kind,
            attributes,
            start,
            end,
            semantic,
            ..
        } => HtmlNode::Element {
            kind: *kind,
            attributes: attributes.clone(),
            children: Vec::new(),
            interaction: None,
            start: *start,
            end: *end,
            semantic: semantic.clone(),
        },
        HtmlNode::Text { .. } => node.clone(),
    }
}

fn text_document(
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
    if style.face.is_some() || style.color.is_some() || style.button_color.is_some() {
        let mut attributes = Vec::new();
        if let Some(face) = style.face {
            attributes.push(HtmlAttribute {
                name: "face".into(),
                value: face.into(),
            });
        }
        if let Some(color) = style.color {
            attributes.push(HtmlAttribute {
                name: "color".into(),
                value: format!("#{color:06X}"),
            });
        }
        if let Some(color) = style.button_color {
            attributes.push(HtmlAttribute {
                name: "bcolor".into(),
                value: format!("#{color:06X}"),
            });
        }
        node = wrap(
            node,
            HtmlElementKind::Font,
            attributes,
            HtmlElementSemantic::Font {
                face: style.face.map(str::to_owned),
                color: style.color,
                button_color: style.button_color,
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
