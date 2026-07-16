use era_runtime_protocol::{
    AudioState, CellAlignment, Color, DisplayLine, DisplayRun, InputWait, InteractionToken,
    LineAlignment, MediaPlacement, PresentationSettings, PresentationSnapshot, RunLayout,
    SeparatorRole, Shape, SystemTextArgument, SystemTextKey, SystemTextRef, TextStyle,
};
use erabasic_vm::VmValue;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct PresentationModel {
    revision: u64,
    title: String,
    lines: Vec<DisplayLine>,
    pending_runs: Vec<DisplayRun>,
    pending_temporary: bool,
    input_wait: Option<InputWait>,
    next_line: u64,
    settings: PresentationSettings,
    project_column_cells: bool,
    project_separators: bool,
    project_html: bool,
    project_graphics: bool,
    project_audio: bool,
    current_style: TextStyle,
    current_alignment: LineAlignment,
    backgrounds: Vec<MediaPlacement>,
    audio: Vec<AudioState>,
    print_c_per_line: u32,
    print_c_length: u32,
    pending_column_cells: u32,
}

impl Default for PresentationModel {
    fn default() -> Self {
        Self {
            revision: 0,
            title: String::new(),
            lines: Vec::new(),
            pending_runs: Vec::new(),
            pending_temporary: false,
            input_wait: None,
            next_line: 1,
            settings: PresentationSettings {
                drawable_width_millipixels: 760_000,
                line_height_millipixels: 19_000,
                background: Color {
                    red: 0,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                },
                button_focus_foreground: Color {
                    red: 255,
                    green: 255,
                    blue: 0,
                    alpha: 255,
                },
            },
            project_column_cells: true,
            project_separators: true,
            project_html: false,
            project_graphics: false,
            project_audio: false,
            current_style: default_style(),
            current_alignment: LineAlignment::Left,
            backgrounds: Vec::new(),
            audio: Vec::new(),
            print_c_per_line: 3,
            print_c_length: 25,
            pending_column_cells: 0,
        }
    }
}

impl PresentationModel {
    pub(crate) fn rebind_interactions(
        &mut self,
        tokens: &BTreeMap<InteractionToken, InteractionToken>,
        waits: &BTreeMap<u64, u64>,
    ) {
        if let Some(wait) = &mut self.input_wait {
            if let Some(rebound) = waits.get(&wait.wait_id) {
                wait.wait_id = *rebound;
            }
            if let Some(rebound) = tokens.get(&wait.submission_token) {
                wait.submission_token = *rebound;
            }
        }
        for line in &mut self.lines {
            rebind_runs(&mut line.runs, tokens);
        }
        rebind_runs(&mut self.pending_runs, tokens);
        self.bump();
    }
    pub(crate) fn set_title(&mut self, title: String) {
        self.title = title;
        self.bump();
    }

    /// Deterministic log projection used by OUTPUTLOG. Device, window and patch
    /// directory details from the UI-coupled reference implementation are omitted.
    pub(crate) fn log_text(&self, hide_info: bool) -> String {
        let mut output = String::new();
        if !hide_info {
            output.push_str("RustyEra Runtime\r\n");
            output.push_str("Game: ");
            output.push_str(&self.title);
            output.push_str("\r\nLog:\r\n");
        }
        for line in &self.lines {
            for run in &line.runs {
                append_log_run(&mut output, run);
            }
            output.push_str("\r\n");
        }
        for run in &self.pending_runs {
            append_log_run(&mut output, run);
        }
        output
    }

    pub(crate) fn append_text(&mut self, text: String, temporary: bool) {
        self.append_print_text(text, temporary, true);
    }

    pub(crate) fn last_line_is_temporary(&self) -> bool {
        self.lines.last().is_some_and(|line| line.temporary)
            || (!self.pending_runs.is_empty() && self.pending_temporary)
    }

    pub(crate) fn last_line_is_empty(&self) -> bool {
        if !self.pending_runs.is_empty() {
            return self.pending_runs.iter().all(run_is_empty);
        }
        self.lines
            .last()
            .is_none_or(|line| line.runs.iter().all(run_is_empty))
    }

    /// Delete canonical logical lines, including an uncommitted current line first.
    /// This models the small console-editing subset used by reference system flows.
    pub(crate) fn delete_last_lines(&mut self, mut count: usize) {
        if count != 0 && !self.pending_runs.is_empty() {
            self.pending_runs.clear();
            self.pending_temporary = false;
            self.pending_column_cells = 0;
            count -= 1;
        }
        let keep = self.lines.len().saturating_sub(count);
        self.lines.truncate(keep);
        self.bump();
    }

    pub(crate) fn replace_last_temporary(&mut self, text: String) {
        self.delete_last_lines(1);
        self.append_text(text, true);
    }

    pub(crate) fn append_system_text(
        &mut self,
        text: String,
        key: SystemTextKey,
        arguments: Vec<SystemTextArgument>,
        temporary: bool,
    ) {
        self.pending_temporary |= temporary;
        let mut run = self.text_run(text);
        if let DisplayRun::Text { system_text, .. } = &mut run {
            *system_text = Some(SystemTextRef { key, arguments });
        }
        self.pending_runs.push(run);
        self.bump();
        self.commit_line();
    }

    /// Append PRINT-family text to the canonical logical line buffer.
    pub(crate) fn append_print_text(&mut self, text: String, temporary: bool, commit: bool) {
        self.pending_temporary |= temporary;
        self.pending_runs.push(self.text_run(text));
        self.bump();
        if commit {
            self.commit_line();
        }
    }

    pub(crate) fn append_column_cell(&mut self, text: String, alignment: CellAlignment) {
        let content = vec![self.text_run(text)];
        self.pending_runs.push(DisplayRun::ColumnCell {
            content,
            alignment,
            // Emuera's default PrintCLength is 25. This is layout intent, not padding.
            preferred_columns: self.print_c_length,
        });
        self.pending_column_cells = self.pending_column_cells.saturating_add(1);
        self.bump();
        if self.pending_column_cells >= self.print_c_per_line {
            self.commit_line();
        }
    }

    pub(crate) fn append_separator(&mut self, pattern: String) {
        if !self.pending_runs.is_empty() {
            self.commit_line();
        }
        self.pending_runs.push(DisplayRun::Separator {
            pattern,
            role: SeparatorRole::Rule,
        });
        self.bump();
        self.commit_line();
    }

    pub(crate) fn append_html(&mut self, markup: String) {
        self.pending_runs.push(DisplayRun::Html { markup });
        self.bump();
        self.commit_line();
    }

    pub(crate) fn append_image(&mut self, resource_id: String, alt_text: Option<String>) {
        self.pending_runs.push(DisplayRun::Image {
            placement: MediaPlacement {
                resource_id,
                x_millipixels: 0,
                y_millipixels: 0,
                width_millipixels: 0,
                height_millipixels: self.settings.line_height_millipixels,
                depth: 0,
                opacity_millionths: 1_000_000,
                revision: self.revision.saturating_add(1),
            },
            alt_text,
        });
        self.bump();
    }

    pub(crate) fn append_rectangle(&mut self, parameters: Vec<i64>) {
        self.pending_runs.push(DisplayRun::Shape {
            shape: Shape {
                kind: "rectangle".into(),
                parameters,
            },
            layout: RunLayout {
                x_millipixels: 0,
                y_millipixels: 0,
                width_millipixels: 0,
                height_millipixels: self.settings.line_height_millipixels,
                depth: 0,
            },
        });
        self.bump();
    }

    pub(crate) fn set_alignment(&mut self, alignment: LineAlignment) {
        self.current_alignment = alignment;
        self.bump();
    }

    pub(crate) fn set_font_style(&mut self, bits: i64) {
        self.current_style.bold = bits & 1 != 0;
        self.current_style.italic = bits & 2 != 0;
        self.current_style.strikeout = bits & 4 != 0;
        self.current_style.underline = bits & 8 != 0;
        self.bump();
    }

    pub(crate) fn set_font(&mut self, family: Option<String>) {
        self.current_style.font_family = family.filter(|value| !value.is_empty());
        self.bump();
    }

    pub(crate) fn set_foreground(&mut self, rgb: i64) {
        self.current_style.foreground = rgb_color(rgb);
        self.bump();
    }

    pub(crate) fn set_background(&mut self, rgb: i64) {
        self.settings.background = rgb_color(rgb);
        self.bump();
    }

    pub(crate) fn set_audio(&mut self, resource_id: String, bgm: bool, playing: bool) {
        let channel_id = u64::from(bgm);
        self.audio.retain(|state| state.channel_id != channel_id);
        if playing {
            self.audio.push(AudioState {
                channel_id,
                resource_id,
                repeat_count: if bgm { -1 } else { 1 },
                volume_millionths: 1_000_000,
                playing: true,
                revision: self.revision.saturating_add(1),
            });
        }
        self.bump();
    }

    #[allow(clippy::fn_params_excessive_bools)]
    pub(crate) fn set_projection(
        &mut self,
        column_cells: bool,
        separators: bool,
        html: bool,
        graphics: bool,
        audio: bool,
    ) {
        self.project_column_cells = column_cells;
        self.project_separators = separators;
        self.project_html = html;
        self.project_graphics = graphics;
        self.project_audio = audio;
    }

    pub(crate) fn configure_layout(
        &mut self,
        width: u32,
        print_c_per_line: u32,
        print_c_length: u32,
    ) {
        self.settings.drawable_width_millipixels = i64::from(width).saturating_mul(1_000);
        self.print_c_per_line = print_c_per_line.max(1);
        self.print_c_length = print_c_length.max(1);
        self.bump();
    }

    fn commit_line(&mut self) {
        let line = DisplayLine {
            line_id: self.next_line,
            temporary: self.pending_temporary,
            logical_line_start: true,
            line_end: true,
            alignment: self.current_alignment,
            layout_width_millipixels: None,
            runs: std::mem::take(&mut self.pending_runs),
        };
        self.pending_temporary = false;
        self.pending_column_cells = 0;
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push(line);
        self.bump();
    }

    fn text_run(&self, text: String) -> DisplayRun {
        DisplayRun::Text {
            text,
            style: self.current_style.clone(),
            layout: RunLayout {
                x_millipixels: 0,
                y_millipixels: 0,
                width_millipixels: 0,
                height_millipixels: self.settings.line_height_millipixels,
                depth: 0,
            },
            system_text: None,
        }
    }

    pub(crate) fn append_system_button(
        &mut self,
        text: String,
        key: SystemTextKey,
        arguments: Vec<SystemTextArgument>,
        token: InteractionToken,
    ) {
        self.append_button_with_system_text(text, token, Some(SystemTextRef { key, arguments }));
    }

    pub(crate) fn append_button(&mut self, text: String, token: InteractionToken) {
        self.append_button_with_system_text(text, token, None);
    }

    fn append_button_with_system_text(
        &mut self,
        text: String,
        token: InteractionToken,
        system_text: Option<SystemTextRef>,
    ) {
        let layout = RunLayout {
            x_millipixels: 0,
            y_millipixels: 0,
            width_millipixels: 0,
            height_millipixels: self.settings.line_height_millipixels,
            depth: 0,
        };
        let line = DisplayLine {
            line_id: self.next_line,
            temporary: false,
            logical_line_start: true,
            line_end: true,
            alignment: self.current_alignment,
            layout_width_millipixels: None,
            runs: vec![DisplayRun::Button {
                runs: vec![DisplayRun::Text {
                    text,
                    style: self.current_style.clone(),
                    layout,
                    system_text,
                }],
                token,
                title: None,
                layout,
                hover_style: None,
            }],
        };
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push(line);
        self.bump();
    }

    pub(crate) fn set_wait(&mut self, wait: Option<InputWait>) {
        self.input_wait = wait;
        self.bump();
    }

    pub(crate) fn snapshot(&self) -> PresentationSnapshot {
        let mut lines = self.lines.clone();
        if !self.pending_runs.is_empty() {
            lines.push(DisplayLine {
                line_id: self.next_line,
                temporary: self.pending_temporary,
                logical_line_start: true,
                line_end: false,
                alignment: self.current_alignment,
                layout_width_millipixels: None,
                runs: self.pending_runs.clone(),
            });
        }
        project_lines(
            &mut lines,
            self.project_column_cells,
            self.project_separators,
            self.settings.line_height_millipixels,
            self.project_html,
            self.project_graphics,
        );
        PresentationSnapshot {
            revision: self.revision,
            title: self.title.clone(),
            lines,
            backgrounds: if self.project_graphics {
                self.backgrounds.clone()
            } else {
                Vec::new()
            },
            audio: if self.project_audio {
                self.audio.clone()
            } else {
                Vec::new()
            },
            input_wait: self.input_wait.clone(),
            settings: self.settings.clone(),
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
}

fn rebind_runs(runs: &mut [DisplayRun], tokens: &BTreeMap<InteractionToken, InteractionToken>) {
    for run in runs {
        match run {
            DisplayRun::Button { runs, token, .. } => {
                if let Some(rebound) = tokens.get(token) {
                    *token = *rebound;
                }
                rebind_runs(runs, tokens);
            }
            DisplayRun::ColumnCell { content, .. } => rebind_runs(content, tokens),
            _ => {}
        }
    }
}

fn append_log_run(output: &mut String, run: &DisplayRun) {
    match run {
        DisplayRun::Text { text, .. } => output.push_str(text),
        DisplayRun::Button { runs, .. } => {
            for run in runs {
                append_log_run(output, run);
            }
        }
        DisplayRun::Html { markup } => output.push_str(markup),
        DisplayRun::Image { alt_text, .. } => {
            if let Some(text) = alt_text {
                output.push_str(text);
            }
        }
        DisplayRun::Shape { .. } => {}
        DisplayRun::ColumnCell { content, .. } => {
            for run in content {
                append_log_run(output, run);
            }
            output.push(' ');
        }
        DisplayRun::Separator { pattern, .. } => output.push_str(pattern),
    }
}

fn run_is_empty(run: &DisplayRun) -> bool {
    match run {
        DisplayRun::Text { text, .. } => text.is_empty(),
        DisplayRun::Button { runs, .. } | DisplayRun::ColumnCell { content: runs, .. } => {
            runs.iter().all(run_is_empty)
        }
        DisplayRun::Separator { pattern, .. } => pattern.is_empty(),
        _ => false,
    }
}

#[allow(clippy::fn_params_excessive_bools)]
fn project_lines(
    lines: &mut [DisplayLine],
    cells: bool,
    separators: bool,
    line_height: i64,
    html: bool,
    graphics: bool,
) {
    for line in lines {
        let mut projected = Vec::new();
        for run in std::mem::take(&mut line.runs) {
            match run {
                DisplayRun::ColumnCell { content, .. } if !cells => {
                    if !projected.is_empty() {
                        projected.push(plain_text(" ".into(), line_height));
                    }
                    projected.extend(content);
                }
                DisplayRun::Separator { pattern, .. } if !separators => {
                    // A fixed 75-column projection is deterministic and independent of viewport.
                    let pattern = if pattern.is_empty() { "-" } else { &pattern };
                    projected.push(plain_text(
                        pattern.repeat(75).chars().take(75).collect(),
                        line_height,
                    ));
                }
                DisplayRun::Html { markup } if !html => {
                    projected.push(plain_text(strip_markup(&markup), line_height));
                }
                DisplayRun::Image { alt_text, .. } if !graphics => {
                    if let Some(text) = alt_text {
                        projected.push(plain_text(text, line_height));
                    }
                }
                DisplayRun::Shape { .. } if !graphics => {}
                other => projected.push(other),
            }
        }
        line.runs = projected;
    }
}

fn strip_markup(markup: &str) -> String {
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

fn rgb_color(value: i64) -> Color {
    let value = u32::try_from(value).unwrap_or_default();
    Color {
        red: ((value >> 16) & 0xff) as u8,
        green: ((value >> 8) & 0xff) as u8,
        blue: (value & 0xff) as u8,
        alpha: 255,
    }
}

fn plain_text(text: String, line_height: i64) -> DisplayRun {
    DisplayRun::Text {
        text,
        style: default_style(),
        layout: RunLayout {
            x_millipixels: 0,
            y_millipixels: 0,
            width_millipixels: 0,
            height_millipixels: line_height,
            depth: 0,
        },
        system_text: None,
    }
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
    if pattern.is_empty() {
        return Err("GETLINESTR pattern must not be empty");
    }
    let graphemes: Vec<_> = pattern.graphemes(true).collect();
    let widths: Vec<_> = graphemes
        .iter()
        .map(|grapheme| UnicodeWidthStr::width(*grapheme))
        .collect();
    if widths.iter().all(|width| *width == 0) {
        return Err("GETLINESTR pattern must have positive logical width");
    }
    let mut result = String::new();
    let mut used: usize = 0;
    'fill: loop {
        let before = used;
        for (grapheme, width) in graphemes.iter().zip(&widths) {
            if used.saturating_add(*width) > columns {
                break 'fill;
            }
            result.push_str(grapheme);
            used = used.saturating_add(*width);
        }
        if used == before || used >= columns {
            break;
        }
    }
    Ok(result)
}

fn default_style() -> TextStyle {
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
        font_family: None,
        font_millipoints: 18_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_column_cells_share_the_pending_logical_line() {
        let mut model = PresentationModel::default();
        model.append_column_cell("A".into(), CellAlignment::Right);
        model.append_column_cell("B".into(), CellAlignment::Left);
        let pending = model.snapshot();
        assert_eq!(pending.lines.len(), 1);
        assert!(!pending.lines[0].line_end);
        assert_eq!(pending.lines[0].runs.len(), 2);

        model.append_print_text("done".into(), false, true);
        let committed = model.snapshot();
        assert_eq!(committed.lines.len(), 1);
        assert!(committed.lines[0].line_end);
        assert_eq!(committed.lines[0].runs.len(), 3);
    }

    #[test]
    fn plain_projection_keeps_cell_content_and_inserts_one_ascii_space() {
        let mut model = PresentationModel::default();
        model.set_projection(false, false, false, false, false);
        model.append_column_cell("A".into(), CellAlignment::Right);
        model.append_column_cell("B".into(), CellAlignment::Right);
        let snapshot = model.snapshot();
        assert_eq!(snapshot.lines[0].runs.len(), 3);
        assert!(matches!(
            &snapshot.lines[0].runs[1],
            DisplayRun::Text { text, .. } if text == " "
        ));
    }

    #[test]
    fn separator_flushes_existing_text_to_an_independent_line() {
        let mut model = PresentationModel::default();
        model.append_print_text("prefix".into(), false, false);
        model.append_separator("=".into());
        let snapshot = model.snapshot();
        assert_eq!(snapshot.lines.len(), 2);
        assert!(matches!(
            &snapshot.lines[1].runs[0],
            DisplayRun::Separator { pattern, .. } if pattern == "="
        ));
    }

    #[test]
    fn temporary_empty_lines_can_be_replaced_without_frontend_state() {
        let mut model = PresentationModel::default();
        model.append_text("before".into(), false);
        model.append_text(String::new(), true);
        assert!(model.last_line_is_temporary());
        assert!(model.last_line_is_empty());
        model.replace_last_temporary("invalid".into());
        let snapshot = model.snapshot();
        assert_eq!(snapshot.lines.len(), 2);
        assert!(snapshot.lines[1].temporary);
        assert!(matches!(
            &snapshot.lines[1].runs[0],
            DisplayRun::Text { text, .. } if text == "invalid"
        ));
    }

    #[test]
    fn logical_line_string_uses_width_without_splitting_graphemes() {
        assert_eq!(logical_line_string("界", 5), Ok("界界".into()));
        assert_eq!(
            logical_line_string("e\u{301}", 3),
            Ok("e\u{301}e\u{301}e\u{301}".into())
        );
        assert!(logical_line_string("\u{301}", 10).is_err());
        assert!(logical_line_string("", 10).is_err());
    }

    #[test]
    fn style_and_media_are_canonical_but_capability_projected() {
        let mut model = PresentationModel::default();
        model.set_font_style(1 | 8);
        model.set_alignment(LineAlignment::Center);
        model.append_print_text("styled".into(), false, true);
        model.append_html("<b>fallback</b>".into());
        model.append_image("image.png".into(), Some("image".into()));
        model.set_audio("sound.ogg".into(), false, true);

        let fallback = model.snapshot();
        assert!(fallback.audio.is_empty());
        assert_eq!(fallback.lines[0].alignment, LineAlignment::Center);
        let DisplayRun::Text { style, .. } = &fallback.lines[0].runs[0] else {
            panic!("first run must be text");
        };
        assert!(style.bold);
        assert!(style.underline);
        assert!(matches!(
            &fallback.lines[1].runs[0],
            DisplayRun::Text { text, .. } if text == "fallback"
        ));

        model.set_projection(true, true, true, true, true);
        let rich = model.snapshot();
        assert_eq!(rich.audio.len(), 1);
        assert!(matches!(rich.lines[1].runs[0], DisplayRun::Html { .. }));
        assert!(matches!(rich.lines[2].runs[0], DisplayRun::Image { .. }));
    }
}
