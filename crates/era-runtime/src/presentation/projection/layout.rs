pub(super) fn run_is_empty(run: &DisplayRun) -> bool {
    match run {
        DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => text.is_empty(),
        DisplayRun::Button { runs, .. } | DisplayRun::ColumnCell { content: runs, .. } => {
            runs.iter().all(run_is_empty)
        }
        DisplayRun::Separator { pattern, .. } => pattern.is_empty(),
        _ => false,
    }
}

#[allow(clippy::fn_params_excessive_bools)]
pub(super) fn project_lines(
    lines: &mut [DisplayLine],
    cells: bool,
    separators: bool,
    line_height: i64,
    html: bool,
    graphics: bool,
    character_width_mode: CharacterWidthMode,
) {
    for line in lines {
        line.runs = project_runs(
            std::mem::take(&mut line.runs),
            cells,
            separators,
            line_height,
            html,
            graphics,
            character_width_mode,
        );
    }
}

#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
struct ProjectionOptions {
    cells: bool,
    separators: bool,
    line_height: i64,
    html: bool,
    graphics: bool,
    character_width_mode: CharacterWidthMode,
}

impl ProjectionOptions {
    #[allow(clippy::fn_params_excessive_bools)]
    const fn new(
        cells: bool,
        separators: bool,
        line_height: i64,
        html: bool,
        graphics: bool,
        character_width_mode: CharacterWidthMode,
    ) -> Self {
        Self {
            cells,
            separators,
            line_height,
            html,
            graphics,
            character_width_mode,
        }
    }
}

#[allow(clippy::fn_params_excessive_bools)]
pub(super) fn project_runs(
    runs: Vec<DisplayRun>,
    cells: bool,
    separators: bool,
    line_height: i64,
    html: bool,
    graphics: bool,
    character_width_mode: CharacterWidthMode,
) -> Vec<DisplayRun> {
    let options = ProjectionOptions::new(
        cells,
        separators,
        line_height,
        html,
        graphics,
        character_width_mode,
    );
    let mut projected = Vec::new();
    let mut runs = VecDeque::from(runs);
    while let Some(run) = runs.pop_front() {
        match run {
            DisplayRun::Text {
                text,
                style,
                system_text,
            }
            | DisplayRun::TextLayout {
                text,
                style,
                system_text,
                ..
            } => {
                let suppress_alignment_space = system_text.is_none()
                    && text_fragment_ends_before_double_vertical_edge(&style, &runs);
                extend_text_layouts(
                    &mut projected,
                    text,
                    style,
                    system_text,
                    suppress_alignment_space,
                    character_width_mode,
                );
            }
            DisplayRun::Button {
                runs,
                token,
                title,
                hover_style,
                value,
                generation,
                enabled,
            } => projected.push(DisplayRun::Button {
                runs: project_runs(
                    runs,
                    cells,
                    separators,
                    line_height,
                    html,
                    graphics,
                    character_width_mode,
                ),
                token,
                title,
                hover_style,
                value,
                generation,
                enabled,
            }),
            DisplayRun::ColumnCell {
                content,
                alignment,
                width,
            } => projected.extend(project_column_cell(content, alignment, width, options)),
            DisplayRun::Separator { pattern, style, .. } if !separators => {
                projected.push(project_separator(&pattern, style, character_width_mode));
            }
            DisplayRun::HtmlDocument { document } if !html => {
                projected.push(projected_plain_text(
                    strip_markup(&erabasic_html::serialize_document(&document)),
                    line_height,
                    character_width_mode,
                ));
            }
            DisplayRun::Image { alt_text, .. } if !graphics => {
                if let Some(text) = alt_text {
                    projected.push(projected_plain_text(
                        text,
                        line_height,
                        character_width_mode,
                    ));
                }
            }
            DisplayRun::Shape { .. } if !graphics => {}
            other => projected.push(other),
        }
    }
    projected
}

fn project_separator(
    pattern: &str,
    style: TextStyle,
    character_width_mode: CharacterWidthMode,
) -> DisplayRun {
    // A fixed 75-column projection is deterministic and independent of viewport.
    let pattern = if pattern.is_empty() { "-" } else { pattern };
    let text = erabasic_vm::logical_line_string_with_mode(pattern, 75, character_width_mode)
        .unwrap_or_default();
    text_layout(text, style, None, character_width_mode)
}

fn project_column_cell(
    content: Vec<DisplayRun>,
    alignment: CellAlignment,
    width: CellWidthIntent,
    options: ProjectionOptions,
) -> Vec<DisplayRun> {
    let content = project_runs(
        content,
        options.cells,
        options.separators,
        options.line_height,
        options.html,
        options.graphics,
        options.character_width_mode,
    );
    if options.cells {
        return vec![DisplayRun::ColumnCell {
            content,
            alignment,
            width,
        }];
    }
    let CellWidthIntent::ProjectColumns(preferred_columns) = width else {
        // A text-only projection has no authoritative font metrics. Keeping the
        // content unpadded is deterministic and does not invent pixel equivalence.
        return content;
    };
    let width = projected_text_width(&content, options.character_width_mode);
    let padding = " ".repeat(
        usize::try_from(preferred_columns)
            .unwrap_or(usize::MAX)
            .saturating_sub(width),
    );
    let mut projected = Vec::new();
    if alignment == CellAlignment::Right && !padding.is_empty() {
        projected.push(projected_plain_text(
            padding.clone(),
            options.line_height,
            options.character_width_mode,
        ));
    }
    projected.extend(content);
    if alignment == CellAlignment::Left && !padding.is_empty() {
        projected.push(projected_plain_text(
            padding,
            options.line_height,
            options.character_width_mode,
        ));
    }
    projected
}

pub(super) fn projected_text_width(
    runs: &[DisplayRun],
    character_width_mode: CharacterWidthMode,
) -> usize {
    runs.iter()
        .map(|run| match run {
            DisplayRun::Text { text, .. } => display_width(text, character_width_mode),
            DisplayRun::TextLayout { columns, .. } => *columns as usize,
            DisplayRun::Button { runs, .. } | DisplayRun::ColumnCell { content: runs, .. } => {
                projected_text_width(runs, character_width_mode)
            }
            DisplayRun::HtmlDocument { document } => emuera_display_width(
                strip_markup(&erabasic_html::serialize_document(document)).as_str(),
            ),
            DisplayRun::Image { alt_text, .. } => {
                alt_text.as_deref().map_or(0, emuera_display_width)
            }
            DisplayRun::Shape { .. } | DisplayRun::Separator { .. } | DisplayRun::Space { .. } => 0,
        })
        .sum()
}

pub(super) fn strip_markup(markup: &str) -> String {
    let mut output = String::new();
    let mut in_tag = false;
    for character in markup.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    output
}

pub(super) fn rgb_color(value: i64) -> Color {
    let value = u32::try_from(value).unwrap_or_default();
    Color {
        red: ((value >> 16) & 0xff) as u8,
        green: ((value >> 8) & 0xff) as u8,
        blue: (value & 0xff) as u8,
        alpha: 255,
    }
}

pub(super) fn color_rgb(color: Color) -> i64 {
    (i64::from(color.red) << 16) | (i64::from(color.green) << 8) | i64::from(color.blue)
}

pub(super) fn plain_text(text: String, _line_height: i64) -> DisplayRun {
    DisplayRun::Text {
        text,
        style: default_style(),
        system_text: None,
    }
}

fn projected_plain_text(
    text: String,
    line_height: i64,
    character_width_mode: CharacterWidthMode,
) -> DisplayRun {
    let DisplayRun::Text {
        text,
        style,
        system_text,
    } = plain_text(text, line_height)
    else {
        unreachable!("plain_text always returns text")
    };
    text_layout(text, style, system_text, character_width_mode)
}

fn text_layout(
    text: String,
    style: TextStyle,
    system_text: Option<era_runtime_protocol::SystemTextRef>,
    character_width_mode: CharacterWidthMode,
) -> DisplayRun {
    let columns = u32::try_from(display_width(&text, character_width_mode)).unwrap_or(u32::MAX);
    DisplayRun::TextLayout {
        text,
        style,
        system_text,
        columns,
    }
}

fn text_fragment_ends_before_double_vertical_edge(
    style: &TextStyle,
    remaining: &VecDeque<DisplayRun>,
) -> bool {
    for run in remaining {
        match run {
            DisplayRun::Text {
                text,
                style: next_style,
                system_text: None,
            }
            | DisplayRun::TextLayout {
                text,
                style: next_style,
                system_text: None,
                ..
            } if next_style == style => {
                if !text.is_empty() {
                    return false;
                }
            }
            DisplayRun::Text {
                text,
                system_text: None,
                ..
            }
            | DisplayRun::TextLayout {
                text,
                system_text: None,
                ..
            } => return text.starts_with('║'),
            _ => return false,
        }
    }
    false
}

fn extend_text_layouts(
    output: &mut Vec<DisplayRun>,
    text: String,
    style: TextStyle,
    system_text: Option<era_runtime_protocol::SystemTextRef>,
    suppress_trailing_ascii_spaces: bool,
    character_width_mode: CharacterWidthMode,
) {
    if text.is_empty() {
        output.push(text_layout(text, style, system_text, character_width_mode));
        return;
    }
    let mut system_text = system_text;
    let trailing_ascii_spaces = if suppress_trailing_ascii_spaces {
        text.chars()
            .rev()
            .take_while(|character| *character == ' ')
            .count()
    } else {
        0
    };
    let grapheme_count = text.graphemes(true).count();
    output.extend(text.graphemes(true).enumerate().map(|(index, grapheme)| {
        let mut layout = text_layout(
            grapheme.to_owned(),
            style.clone(),
            system_text.take(),
            character_width_mode,
        );
        // eraTW uses a separately styled ASCII spacer as an alignment marker
        // before the shrine's double vertical edge. It must not create a
        // half-cell between the full-width map columns.
        if grapheme == " " && index >= grapheme_count.saturating_sub(trailing_ascii_spaces) {
            let DisplayRun::TextLayout { columns, .. } = &mut layout else {
                unreachable!("text_layout always returns projected text")
            };
            *columns = 0;
        }
        layout
    }));
}

pub(crate) fn display_value(value: &VmValue) -> String {
    match value {
        VmValue::Integer(value) => value.to_string(),
        VmValue::String(value) => value.clone(),
        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => "<place>".into(),
    }
}

pub(super) fn default_style() -> TextStyle {
    TextStyle::default()
}
