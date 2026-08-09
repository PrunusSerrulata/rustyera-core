use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use era_runtime_protocol::{
    CellAlignment, Color, DisplayLine, DisplayRun, InteractionToken, LogicalLength,
    PresentationLength, ProtocolValue, TextStyle,
};
use erabasic_vm::{VmValue, emuera_display_width};
use unicode_segmentation::UnicodeSegmentation as _;

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

pub(super) fn enabled_button_value(run: &DisplayRun, token: InteractionToken) -> Option<VmValue> {
    match run {
        DisplayRun::Button {
            runs,
            token: button_token,
            value,
            enabled,
            ..
        } => {
            if *enabled && *button_token == token {
                return Some(match value {
                    ProtocolValue::Integer(value) => VmValue::Integer(*value),
                    ProtocolValue::String(value) => VmValue::String(value.clone()),
                    ProtocolValue::Boolean(value) => VmValue::Integer(i64::from(*value)),
                    ProtocolValue::Bytes(_) => VmValue::String(String::new()),
                });
            }
            runs.iter()
                .rev()
                .find_map(|run| enabled_button_value(run, token))
        }
        DisplayRun::ColumnCell { content, .. } => content
            .iter()
            .rev()
            .find_map(|run| enabled_button_value(run, token)),
        DisplayRun::HtmlDocument { document } => enabled_html_button_value(&document.nodes, token),
        _ => None,
    }
}

pub(super) fn enabled_html_button_value(
    nodes: &[erabasic_html::HtmlNode],
    token: InteractionToken,
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
        enabled_html_button_value(children, token)
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

pub(super) fn append_log_run(output: &mut String, run: &DisplayRun) {
    match run {
        DisplayRun::Text { text, .. } | DisplayRun::TextLayout { text, .. } => {
            output.push_str(text);
        }
        DisplayRun::Button { runs, .. } => {
            for run in runs {
                append_log_run(output, run);
            }
        }
        DisplayRun::HtmlDocument { document } => {
            output.push_str(&erabasic_html::serialize_document(document));
        }
        DisplayRun::Image { alt_text, .. } => {
            if let Some(text) = alt_text {
                output.push_str(text);
            }
        }
        DisplayRun::Shape { .. } | DisplayRun::Space { .. } => {}
        DisplayRun::ColumnCell { content, .. } => {
            for run in content {
                append_log_run(output, run);
            }
            output.push(' ');
        }
        DisplayRun::Separator { pattern, .. } => output.push_str(pattern),
    }
}

#[allow(clippy::too_many_lines)]
pub(super) fn append_html_run(output: &mut String, run: &DisplayRun, line_height: LogicalLength) {
    match run {
        DisplayRun::Text { text, style, .. } | DisplayRun::TextLayout { text, style, .. } => {
            let mut value = erabasic_html::escape(text);
            if style.strikeout {
                value = format!("<s>{value}</s>");
            }
            if style.underline {
                value = format!("<u>{value}</u>");
            }
            if style.italic {
                value = format!("<i>{value}</i>");
            }
            if style.bold {
                value = format!("<b>{value}</b>");
            }
            output.push_str(&value);
        }
        DisplayRun::Button {
            runs, value, title, ..
        } => {
            output.push_str("<button value='");
            let value = match value {
                ProtocolValue::Integer(value) => value.to_string(),
                ProtocolValue::String(value) => value.clone(),
                ProtocolValue::Boolean(value) => i64::from(*value).to_string(),
                ProtocolValue::Bytes(_) => String::new(),
            };
            output.push_str(&erabasic_html::escape(&value));
            if let Some(title) = title {
                output.push_str("' title='");
                output.push_str(&erabasic_html::escape(title));
            }
            output.push_str("'>");
            for run in runs {
                append_html_run(output, run, line_height);
            }
            output.push_str("</button>");
        }
        DisplayRun::HtmlDocument { document } => {
            output.push_str(&erabasic_html::serialize_document(document));
        }
        DisplayRun::Image { placement, .. } => {
            output.push_str("<img src='");
            output.push_str(&erabasic_html::escape(&placement.resource_id));
            if let Some(resource) = &placement.hover_resource_id {
                output.push_str("' srcb='");
                output.push_str(&erabasic_html::escape(resource));
            }
            if let Some(resource) = &placement.mask_resource_id {
                output.push_str("' srcm='");
                output.push_str(&erabasic_html::escape(resource));
            }
            for (name, value) in [
                ("height", placement.requested_height.as_ref()),
                ("width", placement.requested_width.as_ref()),
                ("ypos", placement.requested_y.as_ref()),
            ] {
                if let Some(value) = value {
                    output.push_str("' ");
                    output.push_str(name);
                    output.push_str("='");
                    append_presentation_length(output, value, line_height);
                }
            }
            output.push_str("'>");
        }
        DisplayRun::Shape { shape } => {
            output.push_str("<shape type='");
            output.push_str(&erabasic_html::escape(&shape.kind));
            output.push_str("' param='");
            for (index, value) in shape.parameters.iter().enumerate() {
                if index != 0 {
                    output.push_str(", ");
                }
                append_raw_mixed_length(output, value);
            }
            output.push('\'');
            if shape
                .foreground
                .is_some_and(|color| color != default_style().foreground)
            {
                output.push_str(" color='");
                append_html_color(output, shape.foreground.expect("checked foreground"));
                output.push('\'');
            }
            if let Some(background) = shape.background {
                output.push_str(" bcolor='");
                append_html_color(output, background);
                output.push('\'');
            }
            output.push('>');
        }
        DisplayRun::ColumnCell { content, .. } => {
            for run in content {
                append_html_run(output, run, line_height);
            }
        }
        DisplayRun::Separator { pattern, .. } => {
            output.push_str(&erabasic_html::escape(pattern));
        }
        DisplayRun::Space { width } => {
            output.push_str("<shape type='space' param='");
            append_raw_mixed_length(output, width);
            output.push_str("'>");
        }
    }
}

pub(super) fn append_html_color(output: &mut String, color: Color) {
    output.push('#');
    let _ = write!(
        output,
        "{:02X}{:02X}{:02X}",
        color.red, color.green, color.blue
    );
}

pub(super) fn append_presentation_length(
    output: &mut String,
    value: &PresentationLength,
    line_height: LogicalLength,
) {
    match value {
        PresentationLength::Logical(LogicalLength(value)) => {
            output.push_str(&(value / 1_000).to_string());
            output.push_str("px");
        }
        PresentationLength::FontHeightHundredths(value) => {
            let pixels = value.saturating_mul(line_height.0) / 100_000;
            output.push_str(&pixels.to_string());
        }
    }
}

pub(super) fn append_raw_mixed_length(output: &mut String, value: &PresentationLength) {
    match value {
        PresentationLength::Logical(LogicalLength(value)) => {
            output.push_str(&(value / 1_000).to_string());
            output.push_str("px");
        }
        PresentationLength::FontHeightHundredths(value) => output.push_str(&value.to_string()),
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
) {
    for line in lines {
        line.runs = project_runs(
            std::mem::take(&mut line.runs),
            cells,
            separators,
            line_height,
            html,
            graphics,
        );
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
) -> Vec<DisplayRun> {
    let mut projected = Vec::new();
    for run in runs {
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
            } => extend_text_layouts(&mut projected, text, style, system_text),
            DisplayRun::Button {
                runs,
                token,
                title,
                hover_style,
                value,
                generation,
                enabled,
            } => projected.push(DisplayRun::Button {
                runs: project_runs(runs, cells, separators, line_height, html, graphics),
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
                preferred_columns,
            } => {
                let content = project_runs(content, cells, separators, line_height, html, graphics);
                if cells {
                    projected.push(DisplayRun::ColumnCell {
                        content,
                        alignment,
                        preferred_columns,
                    });
                } else {
                    let width = projected_text_width(&content);
                    let padding = " ".repeat(
                        usize::try_from(preferred_columns)
                            .unwrap_or(usize::MAX)
                            .saturating_sub(width),
                    );
                    if alignment == CellAlignment::Right && !padding.is_empty() {
                        projected.push(projected_plain_text(padding.clone(), line_height));
                    }
                    projected.extend(content);
                    if alignment == CellAlignment::Left && !padding.is_empty() {
                        projected.push(projected_plain_text(padding, line_height));
                    }
                }
            }
            DisplayRun::Separator { pattern, .. } if !separators => {
                // A fixed 75-column projection is deterministic and independent of viewport.
                let pattern = if pattern.is_empty() { "-" } else { &pattern };
                let text = logical_line_string(pattern, 75).unwrap_or_default();
                projected.push(projected_plain_text(text, line_height));
            }
            DisplayRun::HtmlDocument { document } if !html => {
                projected.push(projected_plain_text(
                    strip_markup(&erabasic_html::serialize_document(&document)),
                    line_height,
                ));
            }
            DisplayRun::Image { alt_text, .. } if !graphics => {
                if let Some(text) = alt_text {
                    projected.push(projected_plain_text(text, line_height));
                }
            }
            DisplayRun::Shape { .. } if !graphics => {}
            other => projected.push(other),
        }
    }
    projected
}

pub(super) fn projected_text_width(runs: &[DisplayRun]) -> usize {
    runs.iter()
        .map(|run| match run {
            DisplayRun::Text { text, .. } => emuera_display_width(text),
            DisplayRun::TextLayout { columns, .. } => *columns as usize,
            DisplayRun::Button { runs, .. } | DisplayRun::ColumnCell { content: runs, .. } => {
                projected_text_width(runs)
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

fn projected_plain_text(text: String, line_height: i64) -> DisplayRun {
    let DisplayRun::Text {
        text,
        style,
        system_text,
    } = plain_text(text, line_height)
    else {
        unreachable!("plain_text always returns text")
    };
    text_layout(text, style, system_text)
}

fn text_layout(
    text: String,
    style: TextStyle,
    system_text: Option<era_runtime_protocol::SystemTextRef>,
) -> DisplayRun {
    let columns = u32::try_from(emuera_display_width(&text)).unwrap_or(u32::MAX);
    DisplayRun::TextLayout {
        text,
        style,
        system_text,
        columns,
    }
}

fn extend_text_layouts(
    output: &mut Vec<DisplayRun>,
    text: String,
    style: TextStyle,
    system_text: Option<era_runtime_protocol::SystemTextRef>,
) {
    if text.is_empty() {
        output.push(text_layout(text, style, system_text));
        return;
    }
    let mut system_text = system_text;
    output.extend(
        text.graphemes(true)
            .map(|grapheme| text_layout(grapheme.to_owned(), style.clone(), system_text.take())),
    );
}

pub(crate) fn display_value(value: &VmValue) -> String {
    match value {
        VmValue::Integer(value) => value.to_string(),
        VmValue::String(value) => value.clone(),
        VmValue::IntegerPlace(_) | VmValue::StringPlace(_) => "<place>".into(),
    }
}

/// Repeat a pattern to a deterministic logical-column limit without splitting graphemes.
pub(crate) fn logical_line_string(pattern: &str, columns: usize) -> Result<String, &'static str> {
    erabasic_vm::logical_line_string(pattern, columns)
}

pub(super) fn default_style() -> TextStyle {
    TextStyle {
        foreground: Color {
            red: 192,
            green: 192,
            blue: 192,
            alpha: 255,
        },
        background: None,
        bold: false,
        italic: false,
        underline: false,
        strikeout: false,
        font_family: Some("ＭＳ ゴシック".into()),
        font_millipixels: 18_000,
    }
}
