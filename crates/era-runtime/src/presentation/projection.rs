use std::collections::{BTreeMap, BTreeSet, VecDeque};

use era_runtime_protocol::{
    CellAlignment, CellWidthIntent, Color, DisplayLine, DisplayRun, InteractionToken,
    ProtocolValue, TextStyle,
};
use erabasic_vm::{CharacterWidthMode, VmValue, display_width, emuera_display_width};
use unicode_segmentation::UnicodeSegmentation as _;

mod serialization;
pub(in crate::presentation) use serialization::{append_html_run, append_log_run};
pub(super) use serialization::{append_plain_run, append_printed_html_run};

pub(super) fn auto_button_groups(
    runs: &[DisplayRun],
    plain_runs: &BTreeSet<usize>,
) -> Vec<(usize, usize, String)> {
    let mut groups = Vec::new();
    let mut index = 0;
    while index < runs.len() {
        if plain_runs.contains(&index) || !matches!(runs[index], DisplayRun::Text { .. }) {
            index += 1;
            continue;
        }
        let start = index;
        let mut text = String::new();
        while index < runs.len()
            && !plain_runs.contains(&index)
            && matches!(runs[index], DisplayRun::Text { .. })
        {
            if let DisplayRun::Text { text: value, .. } = &runs[index] {
                text.push_str(value);
            }
            index += 1;
        }
        groups.push((start, index, text));
    }
    groups
}

pub(super) fn auto_button_values(runs: &[DisplayRun], plain_runs: &BTreeSet<usize>) -> Vec<i64> {
    auto_button_groups(runs, plain_runs)
        .into_iter()
        .flat_map(|(_, _, text)| erabasic_html::split_auto_buttons(&text))
        .filter_map(|segment| segment.value)
        .collect()
}

pub(super) fn bind_auto_buttons(
    runs: &mut Vec<DisplayRun>,
    plain_runs: &BTreeSet<usize>,
    tokens: &[InteractionToken],
    generation: u64,
) -> Vec<(InteractionToken, i64)> {
    let groups = auto_button_groups(runs, plain_runs);
    let expected = groups
        .iter()
        .flat_map(|(_, _, text)| erabasic_html::split_auto_buttons(text))
        .filter(|segment| segment.value.is_some())
        .count();
    if expected == 0 || expected != tokens.len() {
        return Vec::new();
    }
    let original = std::mem::take(runs);
    let mut token_iter = tokens.iter().copied();
    let mut bindings = Vec::with_capacity(expected);
    let mut cursor = 0;
    for (start, end, text) in groups {
        runs.extend_from_slice(&original[cursor..start]);
        for segment in erabasic_html::split_auto_buttons(&text) {
            let content = slice_text_runs(&original[start..end], segment.start, segment.end);
            if let Some(value) = segment.value {
                let token = token_iter.next().expect("validated token count");
                runs.push(DisplayRun::Button {
                    runs: content,
                    token,
                    title: None,
                    hover_style: None,
                    value: ProtocolValue::Integer(value),
                    generation,
                    enabled: true,
                });
                bindings.push((token, value));
            } else {
                runs.extend(content);
            }
        }
        cursor = end;
    }
    runs.extend_from_slice(&original[cursor..]);
    bindings
}

pub(super) fn slice_text_runs(runs: &[DisplayRun], start: usize, end: usize) -> Vec<DisplayRun> {
    let mut result = Vec::new();
    let mut cursor = 0;
    for run in runs {
        let DisplayRun::Text {
            text,
            style,
            system_text,
        } = run
        else {
            continue;
        };
        let run_start = cursor;
        let run_end = cursor + text.len();
        cursor = run_end;
        let overlap_start = start.max(run_start);
        let overlap_end = end.min(run_end);
        if overlap_start >= overlap_end {
            continue;
        }
        result.push(DisplayRun::Text {
            text: text[overlap_start - run_start..overlap_end - run_start].to_owned(),
            style: style.clone(),
            system_text: system_text.clone(),
        });
    }
    result
}

pub(super) fn rebind_runs(
    runs: &mut [DisplayRun],
    tokens: &BTreeMap<InteractionToken, InteractionToken>,
) {
    for run in runs {
        match run {
            DisplayRun::Button { runs, token, .. } => {
                if let Some(rebound) = tokens.get(token) {
                    *token = *rebound;
                }
                rebind_runs(runs, tokens);
            }
            DisplayRun::ColumnCell { content, .. } => rebind_runs(content, tokens),
            DisplayRun::HtmlDocument { document } => {
                rebind_html_nodes(&mut document.nodes, tokens);
            }
            _ => {}
        }
    }
}

pub(super) fn disable_old_buttons(runs: &mut [DisplayRun], generation: u64) {
    for run in runs {
        match run {
            DisplayRun::Button {
                runs,
                generation: button_generation,
                enabled,
                ..
            } => {
                *enabled &= *button_generation == generation;
                disable_old_buttons(runs, generation);
            }
            DisplayRun::ColumnCell { content, .. } => {
                disable_old_buttons(content, generation);
            }
            DisplayRun::HtmlDocument { document } => {
                disable_old_html_buttons(&mut document.nodes, generation);
            }
            _ => {}
        }
    }
}

pub(super) fn enabled_button_value(
    run: &DisplayRun,
    token: InteractionToken,
    generation: u64,
) -> Option<VmValue> {
    match run {
        DisplayRun::Button {
            runs,
            token: button_token,
            value,
            generation: button_generation,
            enabled,
            ..
        } => {
            if *enabled && *button_generation == generation && *button_token == token {
                return Some(match value {
                    ProtocolValue::Integer(value) => VmValue::Integer(*value),
                    ProtocolValue::String(value) => VmValue::String(value.clone()),
                    ProtocolValue::Boolean(value) => VmValue::Integer(i64::from(*value)),
                    ProtocolValue::Bytes(_) => VmValue::String(String::new()),
                });
            }
            runs.iter()
                .rev()
                .find_map(|run| enabled_button_value(run, token, generation))
        }
        DisplayRun::ColumnCell { content, .. } => content
            .iter()
            .rev()
            .find_map(|run| enabled_button_value(run, token, generation)),
        DisplayRun::HtmlDocument { document } => {
            enabled_html_button_value(&document.nodes, token, generation)
        }
        _ => None,
    }
}

pub(super) struct ReplayButtonCandidate {
    pub(super) ordinal: usize,
    pub(super) visible_text: String,
    pub(super) title: Option<String>,
    pub(super) alt_text: Option<String>,
}

pub(super) fn find_replay_button(
    runs: &[DisplayRun],
    token: InteractionToken,
    generation: u64,
    ordinal: &mut usize,
) -> Option<ReplayButtonCandidate> {
    for run in runs {
        match run {
            DisplayRun::Button {
                runs,
                token: button_token,
                title,
                generation: button_generation,
                enabled,
                ..
            } => {
                if *enabled && *button_generation == generation {
                    *ordinal = ordinal.saturating_add(1);
                    if *button_token == token {
                        let mut visible_text = String::new();
                        let mut alt_text = None;
                        collect_replay_run_text(runs, &mut visible_text, &mut alt_text);
                        return Some(ReplayButtonCandidate {
                            ordinal: *ordinal,
                            visible_text,
                            title: title.clone(),
                            alt_text,
                        });
                    }
                }
                if let Some(candidate) = find_replay_button(runs, token, generation, ordinal) {
                    return Some(candidate);
                }
            }
            DisplayRun::ColumnCell { content, .. } => {
                if let Some(candidate) = find_replay_button(content, token, generation, ordinal) {
                    return Some(candidate);
                }
            }
            DisplayRun::HtmlDocument { document } => {
                if let Some(candidate) =
                    find_replay_html_button(&document.nodes, token, generation, ordinal)
                {
                    return Some(candidate);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_replay_html_button(
    nodes: &[erabasic_html::HtmlNode],
    token: InteractionToken,
    generation: u64,
    ordinal: &mut usize,
) -> Option<ReplayButtonCandidate> {
    for node in nodes {
        let erabasic_html::HtmlNode::Element {
            children,
            interaction,
            semantic,
            ..
        } = node
        else {
            continue;
        };
        if let Some(interaction) = interaction
            && interaction.enabled
            && interaction.generation == generation
        {
            *ordinal = ordinal.saturating_add(1);
            if interaction.epoch == token.epoch && interaction.id == token.id {
                let mut visible_text = String::new();
                collect_replay_html_text(children, &mut visible_text);
                let title = match semantic {
                    erabasic_html::HtmlElementSemantic::Button { title, .. }
                    | erabasic_html::HtmlElementSemantic::NonButton { title, .. } => title.clone(),
                    _ => None,
                };
                return Some(ReplayButtonCandidate {
                    ordinal: *ordinal,
                    visible_text,
                    title,
                    alt_text: None,
                });
            }
        }
        if let Some(candidate) = find_replay_html_button(children, token, generation, ordinal) {
            return Some(candidate);
        }
    }
    None
}

fn collect_replay_run_text(
    runs: &[DisplayRun],
    visible_text: &mut String,
    alt_text: &mut Option<String>,
) {
    for run in runs {
        match run {
            DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => {
                visible_text.push_str(text);
            }
            DisplayRun::Button { runs, .. } => {
                collect_replay_run_text(runs, visible_text, alt_text);
            }
            DisplayRun::ColumnCell { content, .. } => {
                collect_replay_run_text(content, visible_text, alt_text);
            }
            DisplayRun::HtmlDocument { document } => {
                collect_replay_html_text(&document.nodes, visible_text);
            }
            DisplayRun::Image {
                alt_text: Some(text),
                ..
            } => {
                visible_text.push_str(text);
                if alt_text.is_none() {
                    *alt_text = Some(text.clone());
                }
            }
            _ => {}
        }
    }
}

fn collect_replay_html_text(nodes: &[erabasic_html::HtmlNode], visible_text: &mut String) {
    for node in nodes {
        match node {
            erabasic_html::HtmlNode::Text { text, .. } => visible_text.push_str(text),
            erabasic_html::HtmlNode::Element { children, .. } => {
                collect_replay_html_text(children, visible_text);
            }
        }
    }
}

pub(super) fn enabled_html_button_value(
    nodes: &[erabasic_html::HtmlNode],
    token: InteractionToken,
    generation: u64,
) -> Option<VmValue> {
    nodes.iter().rev().find_map(|node| {
        let erabasic_html::HtmlNode::Element {
            interaction,
            children,
            ..
        } = node
        else {
            return None;
        };
        if let Some(interaction) = interaction
            && interaction.enabled
            && interaction.generation == generation
            && interaction.epoch == token.epoch
            && interaction.id == token.id
        {
            return interaction.integer_value.map(VmValue::Integer).or_else(|| {
                interaction
                    .string_value
                    .as_ref()
                    .map(|value| VmValue::String(value.clone()))
            });
        }
        enabled_html_button_value(children, token, generation)
    })
}

pub(super) fn disable_old_html_buttons(nodes: &mut [erabasic_html::HtmlNode], generation: u64) {
    for node in nodes {
        let erabasic_html::HtmlNode::Element {
            interaction,
            children,
            ..
        } = node
        else {
            continue;
        };
        if let Some(interaction) = interaction {
            interaction.enabled &= interaction.generation == generation;
        }
        disable_old_html_buttons(children, generation);
    }
}

pub(super) fn rebind_html_nodes(
    nodes: &mut [erabasic_html::HtmlNode],
    tokens: &BTreeMap<InteractionToken, InteractionToken>,
) {
    for node in nodes {
        let erabasic_html::HtmlNode::Element {
            interaction,
            children,
            ..
        } = node
        else {
            continue;
        };
        if let Some(value) = interaction {
            let old = InteractionToken {
                epoch: value.epoch,
                id: value.id,
            };
            if let Some(new) = tokens.get(&old) {
                value.epoch = new.epoch;
                value.id = new.id;
            }
        }
        rebind_html_nodes(children, tokens);
    }
}

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
