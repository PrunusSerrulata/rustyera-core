use era_runtime_protocol::{
    AudioState, CellAlignment, Color, DisplayLine, DisplayRun, InputWait, InteractionToken,
    LineAlignment, LogicalLength, MediaPlacement, PresentationDelta, PresentationHistory,
    PresentationHistoryOperation, PresentationLength, PresentationOperation, PresentationSettings,
    PresentationSnapshot, ProtocolValue, RationalOpacity, RedrawState, ResourceReplay,
    SeparatorRole, Shape, SystemTextArgument, SystemTextKey, SystemTextRef, TextStyle,
    TooltipFormat, TooltipSettings,
};
use erabasic_vm::VmValue;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const fn dirty_line_count() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct PresentationModel {
    revision: u64,
    title: String,
    lines: Vec<DisplayLine>,
    history_operations: Vec<PresentationHistoryOperation>,
    pending_runs: Vec<DisplayRun>,
    pending_plain_runs: BTreeSet<usize>,
    last_committed_plain_runs: BTreeSet<usize>,
    pending_temporary: bool,
    input_wait: Option<InputWait>,
    next_line: u64,
    logical_line_count: i64,
    /// The calculated VM variable is synchronized lazily before execution resumes.
    #[serde(skip, default = "dirty_line_count")]
    line_count_dirty: bool,
    settings: PresentationSettings,
    project_column_cells: bool,
    project_separators: bool,
    project_html: bool,
    project_graphics: bool,
    project_audio: bool,
    current_style: TextStyle,
    default_style: TextStyle,
    default_background: Color,
    current_alignment: LineAlignment,
    redraw_enabled: bool,
    button_generation: u64,
    replace_next_temporary: bool,
    html_island: Vec<erabasic_html::HtmlDocument>,
    backgrounds: Vec<MediaPlacement>,
    audio: Vec<AudioState>,
    tooltip: TooltipSettings,
    resources: ResourceReplay,
    print_c_length: u32,
    /// Frontend delivery bookkeeping is transport state, not authoritative game state.
    #[serde(skip)]
    delivery: PresentationDelivery,
}

#[derive(Clone, Debug, Default)]
struct PresentationDelivery {
    revision: Option<u64>,
    history_index: usize,
    pending_line_id: Option<u64>,
    dirty_lines: BTreeSet<u64>,
    dirty: PresentationDirty,
}

#[derive(Clone, Copy, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
struct PresentationDirty {
    title: bool,
    backgrounds: bool,
    audio: bool,
    input_wait: bool,
    settings: bool,
    tooltip: bool,
    resources: bool,
    html_island: bool,
    redraw: bool,
    force_snapshot: bool,
}

pub(crate) enum PresentationUpdate {
    Snapshot(Box<PresentationSnapshot>),
    Delta(PresentationDelta),
}

impl Default for PresentationModel {
    fn default() -> Self {
        Self {
            revision: 0,
            title: String::new(),
            lines: Vec::new(),
            history_operations: Vec::new(),
            pending_runs: Vec::new(),
            pending_plain_runs: BTreeSet::new(),
            last_committed_plain_runs: BTreeSet::new(),
            pending_temporary: false,
            input_wait: None,
            next_line: 1,
            logical_line_count: 0,
            line_count_dirty: true,
            settings: PresentationSettings {
                drawable_width: LogicalLength(760_000),
                line_height: LogicalLength(19_000),
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
                maximum_physical_lines: 5_000,
                prevent_button_wrap: false,
                legacy_nonbutton_wrap: false,
            },
            project_column_cells: true,
            project_separators: true,
            project_html: false,
            project_graphics: false,
            project_audio: false,
            current_style: default_style(),
            default_style: default_style(),
            default_background: rgb_color(0),
            current_alignment: LineAlignment::Left,
            redraw_enabled: true,
            button_generation: 0,
            replace_next_temporary: false,
            html_island: Vec::new(),
            backgrounds: Vec::new(),
            audio: Vec::new(),
            tooltip: TooltipSettings {
                foreground: rgb_color(0),
                background: rgb_color(0x00ff_ffe1),
                delay_ms: 500,
                duration_ms: 5_000,
                font_family: None,
                font_millipoints: 9_000,
                custom: false,
                format: 0,
                images: false,
                normalized_format: TooltipFormat::default(),
            },
            resources: ResourceReplay::default(),
            print_c_length: 25,
            delivery: PresentationDelivery::default(),
        }
    }
}

impl PresentationModel {
    pub(crate) const fn logical_line_count(&self) -> i64 {
        self.logical_line_count
    }

    pub(crate) const fn line_count_is_dirty(&self) -> bool {
        self.line_count_dirty
    }

    pub(crate) fn mark_line_count_synchronized(&mut self) {
        self.line_count_dirty = false;
    }

    pub(crate) fn last_line_auto_button_values(&self) -> Vec<i64> {
        let Some(line) = self.lines.last() else {
            return Vec::new();
        };
        auto_button_values(&line.runs, &self.last_committed_plain_runs)
    }

    pub(crate) fn pending_auto_button_values(&self) -> Vec<i64> {
        auto_button_values(&self.pending_runs, &self.pending_plain_runs)
    }

    pub(crate) fn bind_last_line_auto_buttons(
        &mut self,
        tokens: &[InteractionToken],
    ) -> Vec<(InteractionToken, i64)> {
        let Some(line) = self.lines.last_mut() else {
            return Vec::new();
        };
        let bindings = bind_auto_buttons(
            &mut line.runs,
            &self.last_committed_plain_runs,
            tokens,
            self.button_generation,
        );
        self.last_committed_plain_runs.clear();
        if !bindings.is_empty() {
            self.delivery.dirty_lines.insert(line.line_id);
            self.bump();
        }
        bindings
    }

    pub(crate) fn bind_pending_auto_buttons(
        &mut self,
        tokens: &[InteractionToken],
    ) -> Vec<(InteractionToken, i64)> {
        let bindings = bind_auto_buttons(
            &mut self.pending_runs,
            &self.pending_plain_runs,
            tokens,
            self.button_generation,
        );
        self.pending_plain_runs.clear();
        if !bindings.is_empty() {
            self.bump();
        }
        bindings
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

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
        self.delivery.dirty.force_snapshot = true;
        self.bump();
    }
    pub(crate) fn set_title(&mut self, title: String) {
        self.title = title;
        self.delivery.dirty.title = true;
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
        let physical_count = u32::try_from(count).unwrap_or(u32::MAX);
        if count != 0 && !self.pending_runs.is_empty() {
            self.pending_runs.clear();
            self.pending_temporary = false;
            count -= 1;
        }
        let logical_deletions = i64::try_from(count).unwrap_or(i64::MAX);
        self.logical_line_count = self.logical_line_count.wrapping_sub(logical_deletions);
        self.line_count_dirty = true;
        let keep = self.lines.len().saturating_sub(count);
        self.lines.truncate(keep);
        self.history_operations
            .push(PresentationHistoryOperation::DeletePhysical {
                count: physical_count,
            });
        self.bump();
    }

    pub(crate) fn replace_last_temporary(&mut self, text: String) {
        self.delete_last_lines(1);
        self.append_text(text, true);
    }

    pub(crate) fn print_temporary_line(&mut self, text: String) {
        if !self.pending_runs.is_empty() && self.pending_temporary {
            self.pending_runs.clear();
        } else if self.lines.last().is_some_and(|line| line.temporary) {
            self.lines.pop();
            self.logical_line_count = self.logical_line_count.wrapping_sub(1);
            self.line_count_dirty = true;
        }
        self.replace_next_temporary = true;
        self.append_print_text(text, true, true);
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
        if text.is_empty() {
            if commit {
                self.force_new_line();
            }
            return;
        }
        self.pending_runs.push(self.text_run(text));
        self.bump();
        if commit {
            self.commit_line();
        }
    }

    /// Append text that must remain outside automatic `[value]` button grouping.
    pub(crate) fn append_plain_print_text(&mut self, text: String, temporary: bool, commit: bool) {
        self.pending_temporary |= temporary;
        if text.is_empty() {
            if commit {
                self.force_new_line();
            }
            return;
        }
        self.pending_plain_runs.insert(self.pending_runs.len());
        self.pending_runs.push(self.text_run(text));
        self.bump();
        if commit {
            self.commit_line();
        }
    }

    /// D-suffixed print commands intentionally ignore SETCOLOR while preserving
    /// the remaining canonical style fields.
    pub(crate) fn append_default_color_text(
        &mut self,
        text: String,
        temporary: bool,
        commit: bool,
    ) {
        let foreground = self.current_style.foreground;
        self.current_style.foreground = self.default_style.foreground;
        self.append_print_text(text, temporary, commit);
        self.current_style.foreground = foreground;
    }

    pub(crate) fn append_column_cell(&mut self, text: String, alignment: CellAlignment) {
        let content = vec![self.text_run(text)];
        self.pending_runs.push(DisplayRun::ColumnCell {
            content,
            alignment,
            // Emuera's default PrintCLength is 25. This is layout intent, not padding.
            preferred_columns: self.print_c_length,
        });
        self.bump();
    }

    pub(crate) fn append_default_color_column_cell(
        &mut self,
        text: String,
        alignment: CellAlignment,
    ) {
        let foreground = self.current_style.foreground;
        self.current_style.foreground = self.default_style.foreground;
        self.append_column_cell(text, alignment);
        self.current_style.foreground = foreground;
    }

    pub(crate) fn last_column_auto_button_values(&self) -> Vec<i64> {
        let Some(DisplayRun::ColumnCell { content, .. }) = self.pending_runs.last() else {
            return Vec::new();
        };
        auto_button_values(content, &BTreeSet::new())
    }

    pub(crate) fn bind_last_column_auto_buttons(
        &mut self,
        tokens: &[InteractionToken],
    ) -> Vec<(InteractionToken, i64)> {
        let Some(DisplayRun::ColumnCell { content, .. }) = self.pending_runs.last_mut() else {
            return Vec::new();
        };
        let bindings = bind_auto_buttons(content, &BTreeSet::new(), tokens, self.button_generation);
        if !bindings.is_empty() {
            self.bump();
        }
        bindings
    }

    pub(crate) fn flush_pending_line(&mut self) {
        if !self.pending_runs.is_empty() {
            self.commit_line();
        }
    }

    pub(crate) fn force_new_line(&mut self) {
        if self.pending_runs.is_empty() {
            self.pending_runs.push(self.text_run(String::new()));
            self.bump();
        }
        self.commit_line();
    }

    pub(crate) fn force_default_color_new_line(&mut self) {
        let foreground = self.current_style.foreground;
        self.current_style.foreground = self.default_style.foreground;
        self.force_new_line();
        self.current_style.foreground = foreground;
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

    pub(crate) fn append_html(&mut self, document: erabasic_html::HtmlDocument) {
        if !self.pending_runs.is_empty() {
            self.commit_line();
        }
        let mut current = Vec::new();
        for node in document.nodes {
            match &node {
                erabasic_html::HtmlNode::Element {
                    kind: erabasic_html::HtmlElementKind::Break,
                    ..
                } => {
                    if !current.is_empty() {
                        self.pending_runs.push(DisplayRun::HtmlDocument {
                            document: erabasic_html::HtmlDocument {
                                nodes: std::mem::take(&mut current),
                            },
                        });
                    }
                    self.commit_line();
                }
                erabasic_html::HtmlNode::Element {
                    kind: erabasic_html::HtmlElementKind::Paragraph,
                    attributes,
                    ..
                } => {
                    if !current.is_empty() {
                        self.pending_runs.push(DisplayRun::HtmlDocument {
                            document: erabasic_html::HtmlDocument {
                                nodes: std::mem::take(&mut current),
                            },
                        });
                        self.commit_line();
                    }
                    let previous = self.current_alignment;
                    if let Some(align) = attributes
                        .iter()
                        .find(|attribute| attribute.name == "align")
                        .map(|attribute| attribute.value.to_ascii_lowercase())
                    {
                        self.current_alignment = match align.as_str() {
                            "center" => LineAlignment::Center,
                            "right" => LineAlignment::Right,
                            _ => LineAlignment::Left,
                        };
                    }
                    self.pending_runs.push(DisplayRun::HtmlDocument {
                        document: erabasic_html::HtmlDocument { nodes: vec![node] },
                    });
                    self.commit_line();
                    self.current_alignment = previous;
                }
                _ => current.push(node),
            }
        }
        if !current.is_empty() {
            self.pending_runs.push(DisplayRun::HtmlDocument {
                document: erabasic_html::HtmlDocument { nodes: current },
            });
            self.commit_line();
        }
        self.bump();
    }

    pub(crate) fn append_html_inline(&mut self, document: erabasic_html::HtmlDocument) {
        self.pending_runs
            .push(DisplayRun::HtmlDocument { document });
        self.bump();
    }

    /// Serialize and consume the runtime-owned print buffer.
    ///
    /// Physical line wrapping is deliberately absent here: `HTML_POPPRINTINGSTR`
    /// observes the semantic buffer before it is committed to frontend history.
    pub(crate) fn pop_printing_html(&mut self) -> String {
        if self.pending_runs.is_empty() {
            return String::new();
        }
        let runs = std::mem::take(&mut self.pending_runs);
        self.pending_temporary = false;
        let mut output = String::new();
        for run in &runs {
            append_html_run(&mut output, run, self.settings.line_height);
        }
        self.bump();
        output
    }

    pub(crate) fn append_html_island(&mut self, document: erabasic_html::HtmlDocument) {
        self.html_island.push(document);
        self.delivery.dirty.html_island = true;
        self.bump();
    }

    pub(crate) fn clear_html_island(&mut self) {
        self.html_island.clear();
        self.delivery.dirty.html_island = true;
        self.bump();
    }

    #[cfg(test)]
    pub(crate) fn append_image(&mut self, resource_id: String, alt_text: Option<String>) {
        self.append_image_with_options(resource_id, None, None, None, None, None, alt_text);
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn append_image_with_options(
        &mut self,
        resource_id: String,
        hover_resource_id: Option<String>,
        mask_resource_id: Option<String>,
        requested_width: Option<PresentationLength>,
        requested_height: Option<PresentationLength>,
        requested_y: Option<PresentationLength>,
        alt_text: Option<String>,
    ) {
        self.pending_runs.push(DisplayRun::Image {
            placement: MediaPlacement {
                resource_id,
                x: LogicalLength(0),
                y: LogicalLength(0),
                width: LogicalLength(0),
                height: self.settings.line_height,
                depth: 0,
                opacity: RationalOpacity {
                    numerator: 1,
                    denominator: 1,
                },
                revision: self.revision.saturating_add(1),
                hover_resource_id,
                mask_resource_id,
                requested_width,
                requested_height,
                requested_y,
            },
            alt_text,
        });
        self.bump();
    }

    pub(crate) fn append_shape(
        &mut self,
        kind: impl Into<String>,
        parameters: Vec<PresentationLength>,
    ) {
        self.pending_runs.push(DisplayRun::Shape {
            shape: Shape {
                kind: kind.into(),
                parameters,
                foreground: Some(self.current_style.foreground),
                background: self.current_style.background,
            },
        });
        self.bump();
    }

    pub(crate) fn append_space(&mut self, width: PresentationLength) {
        self.pending_runs.push(DisplayRun::Space { width });
        self.bump();
    }

    pub(crate) fn set_alignment(&mut self, alignment: LineAlignment) {
        self.current_alignment = alignment;
        self.bump();
    }

    pub(crate) const fn line_height(&self) -> LogicalLength {
        self.settings.line_height
    }

    /// Reset the user-controlled console style without changing the console
    /// background, matching EmueraConsole.ResetStyle.
    pub(crate) fn reset_style(&mut self) {
        self.current_style = default_style();
        self.current_alignment = LineAlignment::Left;
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
        self.delivery.dirty.settings = true;
        self.bump();
    }

    pub(crate) fn reset_foreground(&mut self) {
        self.current_style.foreground = self.default_style.foreground;
        self.bump();
    }

    pub(crate) fn reset_background(&mut self) {
        self.settings.background = self.default_background;
        self.delivery.dirty.settings = true;
        self.bump();
    }

    pub(crate) fn set_bold(&mut self, enabled: bool) {
        self.current_style.bold = enabled;
        self.bump();
    }

    pub(crate) fn set_italic(&mut self, enabled: bool) {
        self.current_style.italic = enabled;
        self.bump();
    }

    pub(crate) fn clear_font_style(&mut self) {
        self.set_font_style(0);
    }

    pub(crate) fn set_redraw(&mut self, enabled: bool) {
        self.redraw_enabled = enabled;
        self.delivery.dirty.redraw = true;
        self.bump();
    }

    pub(crate) fn set_button_generation(&mut self, generation: u64) {
        self.button_generation = generation;
        for line in &mut self.lines {
            disable_old_buttons(&mut line.runs, generation);
        }
        disable_old_buttons(&mut self.pending_runs, generation);
        self.history_operations
            .push(PresentationHistoryOperation::SetButtonGeneration { generation });
        self.bump();
    }

    pub(crate) fn redraw_enabled(&self) -> bool {
        self.redraw_enabled
    }

    pub(crate) fn alignment(&self) -> LineAlignment {
        self.current_alignment
    }

    pub(crate) fn foreground_rgb(&self) -> i64 {
        color_rgb(self.current_style.foreground)
    }

    pub(crate) fn background_rgb(&self) -> i64 {
        color_rgb(self.settings.background)
    }

    pub(crate) fn default_foreground_rgb(&self) -> i64 {
        color_rgb(self.default_style.foreground)
    }

    pub(crate) fn default_background_rgb(&self) -> i64 {
        color_rgb(self.default_background)
    }

    pub(crate) fn focus_rgb(&self) -> i64 {
        color_rgb(self.settings.button_focus_foreground)
    }

    pub(crate) fn font(&self) -> String {
        self.current_style.font_family.clone().unwrap_or_default()
    }

    pub(crate) fn style_bits(&self) -> i64 {
        i64::from(self.current_style.bold)
            | (i64::from(self.current_style.italic) << 1)
            | (i64::from(self.current_style.strikeout) << 2)
            | (i64::from(self.current_style.underline) << 3)
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
        self.delivery.dirty.audio = true;
        self.bump();
    }

    pub(crate) fn set_audio_volume(&mut self, bgm: bool, volume: i64) {
        let channel_id = u64::from(bgm);
        let volume = volume.clamp(0, 100);
        for state in &mut self.audio {
            if state.channel_id == channel_id || (!bgm && state.channel_id != 1) {
                state.volume_millionths = u32::try_from(volume).unwrap_or_default() * 10_000;
                state.revision = self.revision.saturating_add(1);
            }
        }
        self.delivery.dirty.audio = true;
        self.bump();
    }

    pub(crate) fn projects_audio(&self) -> bool {
        self.project_audio
    }

    pub(crate) fn set_resource_replay(&mut self, resources: ResourceReplay) {
        self.resources = resources;
        self.delivery.dirty.resources = true;
        self.bump();
    }

    pub(crate) fn add_background(&mut self, resource_id: String, depth: i64, opacity: i64) {
        self.backgrounds.push(MediaPlacement {
            resource_id,
            x: LogicalLength(0),
            y: LogicalLength(0),
            width: self.settings.drawable_width,
            height: LogicalLength(0),
            depth,
            opacity: RationalOpacity {
                numerator: opacity,
                denominator: 255,
            },
            revision: self.revision.saturating_add(1),
            hover_resource_id: None,
            mask_resource_id: None,
            requested_width: None,
            requested_height: None,
            requested_y: None,
        });
        // Stable descending sort matches List.Sort's intended depth layering
        // while retaining insertion order for equal-depth portable replay.
        self.backgrounds
            .sort_by_key(|placement| std::cmp::Reverse(placement.depth));
        self.delivery.dirty.backgrounds = true;
        self.bump();
    }

    pub(crate) fn remove_background(&mut self, resource_id: &str) -> bool {
        let Some(index) = self
            .backgrounds
            .iter()
            .position(|item| item.resource_id == resource_id)
        else {
            return false;
        };
        self.backgrounds.remove(index);
        self.delivery.dirty.backgrounds = true;
        self.bump();
        true
    }

    pub(crate) fn clear_backgrounds(&mut self) {
        self.backgrounds.clear();
        self.delivery.dirty.backgrounds = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_colors(&mut self, foreground: i64, background: i64) {
        self.tooltip.foreground = rgb_color(foreground);
        self.tooltip.background = rgb_color(background);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_delay(&mut self, delay: i64) -> Result<(), &'static str> {
        self.tooltip.delay_ms =
            u32::try_from(delay).map_err(|_| "tooltip delay is out of range")?;
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_duration(&mut self, duration: i64) -> Result<(), &'static str> {
        let duration = u32::try_from(duration).map_err(|_| "tooltip duration is out of range")?;
        self.tooltip.duration_ms = duration.min(i16::MAX.cast_unsigned().into());
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_font(&mut self, family: String) {
        self.tooltip.font_family = (!family.is_empty()).then_some(family);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_font_size(&mut self, points: i64) -> Result<(), &'static str> {
        let points = u32::try_from(points).map_err(|_| "tooltip font size is out of range")?;
        self.tooltip.font_millipoints = points.saturating_mul(1_000);
        self.delivery.dirty.tooltip = true;
        self.bump();
        Ok(())
    }

    pub(crate) fn set_tooltip_custom(&mut self, enabled: bool) {
        self.tooltip.custom = enabled;
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_format(&mut self, format: i64) {
        self.tooltip.format = format;
        self.tooltip.normalized_format = TooltipFormat::from_raw(format);
        self.delivery.dirty.tooltip = true;
        self.bump();
    }

    pub(crate) fn set_tooltip_images(&mut self, enabled: bool) {
        self.tooltip.images = enabled;
        self.delivery.dirty.tooltip = true;
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
        self.delivery.dirty.force_snapshot = true;
    }

    pub(crate) fn configure_project(
        &mut self,
        project: &crate::project::NormalizedProjectSnapshot,
    ) {
        use erabasic_config::ConfigValue;

        let integer = |code| match project.configuration.get_code(code) {
            Some(ConfigValue::Integer(value)) => Some(*value),
            _ => None,
        };
        let boolean = |code| match project.configuration.get_code(code) {
            Some(ConfigValue::Boolean(value)) => Some(*value),
            _ => None,
        };
        let color = |code| match project.configuration.get_code(code) {
            Some(ConfigValue::Color(value)) => Some(*value),
            _ => None,
        };
        let font = match project.configuration.get_code("FontName") {
            Some(ConfigValue::String(value)) => Some(value.clone()),
            _ => None,
        };
        self.settings.drawable_width =
            LogicalLength(i64::from(project.viewport_width).saturating_mul(1_000));
        self.settings.line_height =
            LogicalLength(i64::from(project.line_height).saturating_mul(1_000));
        self.settings.maximum_physical_lines = integer("MaxLog")
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(5_000);
        self.settings.prevent_button_wrap = boolean("ButtonWrap").unwrap_or(false);
        self.settings.legacy_nonbutton_wrap = boolean("CompatiLinefeedAs1739").unwrap_or(false);
        self.default_style.font_family = font;
        self.default_style.font_millipoints = project.font_size.saturating_mul(1_000);
        self.default_style.foreground =
            rgb_color(i64::from(color("ForeColor").unwrap_or(0x00c0_c0c0)));
        self.default_background = rgb_color(i64::from(color("BackColor").unwrap_or(0)));
        self.settings.background = self.default_background;
        self.settings.button_focus_foreground =
            rgb_color(i64::from(color("FocusColor").unwrap_or(0x00ff_ff00)));
        self.current_style = self.default_style.clone();
        self.print_c_length = project.print_c_length.max(1);
        self.delivery.dirty.settings = true;
        self.bump();
    }

    fn commit_line(&mut self) {
        self.last_committed_plain_runs = std::mem::take(&mut self.pending_plain_runs);
        let line = DisplayLine {
            line_id: self.next_line,
            temporary: self.pending_temporary,
            logical_line_start: true,
            line_end: true,
            alignment: self.current_alignment,
            runs: std::mem::take(&mut self.pending_runs),
        };
        self.pending_temporary = false;
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push(line.clone());
        self.logical_line_count = if self.logical_line_count == i64::MAX {
            0
        } else {
            self.logical_line_count + 1
        };
        self.line_count_dirty = true;
        if self.replace_next_temporary {
            self.history_operations
                .push(PresentationHistoryOperation::ReplaceTemporary { line });
            self.replace_next_temporary = false;
        } else {
            self.history_operations
                .push(PresentationHistoryOperation::Append { line });
        }
        self.trim_physical_history();
        self.bump();
    }

    fn text_run(&self, text: String) -> DisplayRun {
        DisplayRun::Text {
            text,
            style: self.current_style.clone(),
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

    pub(crate) fn append_button(
        &mut self,
        text: String,
        value: ProtocolValue,
        token: InteractionToken,
        column_alignment: Option<CellAlignment>,
    ) {
        let button = self.button_run(text, value, token, None);
        if let Some(alignment) = column_alignment {
            self.pending_runs.push(DisplayRun::ColumnCell {
                content: vec![button],
                alignment,
                preferred_columns: self.print_c_length,
            });
        } else {
            self.pending_runs.push(button);
        }
        self.bump();
    }

    fn append_button_with_system_text(
        &mut self,
        text: String,
        token: InteractionToken,
        system_text: Option<SystemTextRef>,
    ) {
        let line = DisplayLine {
            line_id: self.next_line,
            temporary: false,
            logical_line_start: true,
            line_end: true,
            alignment: self.current_alignment,
            runs: vec![self.button_run(
                text,
                ProtocolValue::String(String::new()),
                token,
                system_text,
            )],
        };
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push(line.clone());
        self.logical_line_count = if self.logical_line_count == i64::MAX {
            0
        } else {
            self.logical_line_count + 1
        };
        self.line_count_dirty = true;
        self.history_operations
            .push(PresentationHistoryOperation::Append { line });
        self.trim_physical_history();
        self.bump();
    }

    fn trim_physical_history(&mut self) {
        let maximum = self.settings.maximum_physical_lines as usize;
        let excess = self.lines.len().saturating_sub(maximum);
        if excess == 0 {
            return;
        }
        self.lines.drain(..excess);
        self.history_operations
            .push(PresentationHistoryOperation::TrimPhysical {
                count: u32::try_from(excess).unwrap_or(u32::MAX),
            });
    }

    fn button_run(
        &self,
        text: String,
        value: ProtocolValue,
        token: InteractionToken,
        system_text: Option<SystemTextRef>,
    ) -> DisplayRun {
        DisplayRun::Button {
            runs: vec![DisplayRun::Text {
                text,
                style: self.current_style.clone(),
                system_text,
            }],
            token,
            title: None,
            hover_style: None,
            value,
            generation: self.button_generation,
            enabled: true,
        }
    }

    pub(crate) fn set_wait(&mut self, wait: Option<InputWait>) {
        self.input_wait = wait;
        self.delivery.dirty.input_wait = true;
        self.bump();
    }

    /// Build the smallest lossless frontend update for the current canonical model.
    ///
    /// Full snapshots remain the synchronization boundary. Once a frontend has a
    /// baseline, the hot PRINT path only clones and projects the lines changed since
    /// the previous emission instead of the complete session history.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn next_update(&mut self) -> PresentationUpdate {
        if self.delivery.revision.is_none()
            || self.delivery.history_index > self.history_operations.len()
            || self.delivery.dirty.force_snapshot
        {
            return PresentationUpdate::Snapshot(Box::new(self.snapshot_for_delivery()));
        }

        let mut delivery = std::mem::take(&mut self.delivery);
        let base_revision = delivery
            .revision
            .expect("the snapshot guard establishes a delivery revision");
        let history = self.history_operations[delivery.history_index..].to_vec();
        let mut operations = Vec::with_capacity(history.len().saturating_add(6));

        if delivery.dirty.title {
            operations.push(PresentationOperation::SetTitle {
                title: self.title.clone(),
            });
        }
        if delivery.dirty.settings {
            operations.push(PresentationOperation::SetSettings {
                settings: self.settings.clone(),
            });
        }
        if delivery.dirty.backgrounds {
            operations.push(PresentationOperation::SetBackgrounds {
                backgrounds: if self.project_graphics {
                    self.backgrounds.clone()
                } else {
                    Vec::new()
                },
            });
        }
        if delivery.dirty.audio {
            operations.push(PresentationOperation::SetAudio {
                audio: if self.project_audio {
                    self.audio.clone()
                } else {
                    Vec::new()
                },
            });
        }
        if delivery.dirty.input_wait {
            operations.push(PresentationOperation::SetInputWait {
                input_wait: self.input_wait.clone(),
            });
        }
        if delivery.dirty.tooltip {
            operations.push(PresentationOperation::SetTooltip {
                tooltip: self.tooltip.clone(),
            });
        }
        if delivery.dirty.resources {
            operations.push(PresentationOperation::SetResources {
                resources: if self.project_graphics {
                    self.resources.clone()
                } else {
                    ResourceReplay::default()
                },
            });
        }
        if delivery.dirty.html_island {
            operations.push(PresentationOperation::SetHtmlIsland {
                html_island: self.html_island.clone(),
            });
        }
        if delivery.dirty.redraw {
            operations.push(PresentationOperation::SetRedraw {
                redraw: RedrawState {
                    enabled: self.redraw_enabled,
                },
            });
        }

        for operation in history {
            match operation {
                PresentationHistoryOperation::Append { line } => {
                    let line_id = line.line_id;
                    let line = if delivery.dirty_lines.remove(&line_id) {
                        self.lines
                            .iter()
                            .rev()
                            .find(|current| current.line_id == line_id)
                            .cloned()
                            .unwrap_or(line)
                    } else {
                        line
                    };
                    let line = self.project_line(line);
                    if delivery.pending_line_id == Some(line_id) {
                        operations.push(PresentationOperation::ReplaceLine { line_id, line });
                        delivery.pending_line_id = None;
                    } else {
                        operations.push(PresentationOperation::AppendLine { line });
                    }
                }
                PresentationHistoryOperation::ReplaceTemporary { line } => {
                    let line_id = line.line_id;
                    let line = if delivery.dirty_lines.remove(&line_id) {
                        self.lines
                            .iter()
                            .rev()
                            .find(|current| current.line_id == line_id)
                            .cloned()
                            .unwrap_or(line)
                    } else {
                        line
                    };
                    operations.push(PresentationOperation::DeleteLines { count: 1 });
                    operations.push(PresentationOperation::AppendLine {
                        line: self.project_line(line),
                    });
                    delivery.pending_line_id = None;
                }
                PresentationHistoryOperation::DeletePhysical { count } => {
                    operations.push(PresentationOperation::DeleteLines { count });
                    delivery.pending_line_id = None;
                }
                PresentationHistoryOperation::Clear => {
                    operations.push(PresentationOperation::Clear);
                    delivery.pending_line_id = None;
                }
                PresentationHistoryOperation::SetButtonGeneration { generation } => {
                    operations.push(PresentationOperation::SetButtonGeneration { generation });
                }
                PresentationHistoryOperation::TrimPhysical { count } => {
                    operations.push(PresentationOperation::TrimLines { count });
                    delivery.pending_line_id = None;
                }
            }
        }

        for line_id in std::mem::take(&mut delivery.dirty_lines) {
            if let Some(line) = self
                .lines
                .iter()
                .find(|current| current.line_id == line_id)
                .cloned()
            {
                operations.push(PresentationOperation::ReplaceLine {
                    line_id,
                    line: self.project_line(line),
                });
            }
        }

        let pending_line = self.pending_line();
        match (delivery.pending_line_id, pending_line) {
            (Some(line_id), Some(line)) if line_id == line.line_id => {
                operations.push(PresentationOperation::ReplaceLine {
                    line_id,
                    line: self.project_line(line),
                });
                delivery.pending_line_id = Some(line_id);
            }
            (Some(_), Some(line)) => {
                operations.push(PresentationOperation::DeleteLines { count: 1 });
                delivery.pending_line_id = Some(line.line_id);
                operations.push(PresentationOperation::AppendLine {
                    line: self.project_line(line),
                });
            }
            (None, Some(line)) => {
                delivery.pending_line_id = Some(line.line_id);
                operations.push(PresentationOperation::AppendLine {
                    line: self.project_line(line),
                });
            }
            (Some(_), None) => {
                operations.push(PresentationOperation::DeleteLines { count: 1 });
                delivery.pending_line_id = None;
            }
            (None, None) => {}
        }

        delivery.revision = Some(self.revision);
        // Encoded outbound envelopes own retransmission after this point. Keeping the
        // same semantic edits in the presentation model made long sessions grow forever.
        self.history_operations.clear();
        delivery.history_index = 0;
        delivery.dirty = PresentationDirty::default();
        self.delivery = delivery;
        PresentationUpdate::Delta(PresentationDelta {
            base_revision,
            new_revision: self.revision,
            operations,
        })
    }

    /// Produce an authoritative baseline and advance the per-session delivery cursor.
    pub(crate) fn snapshot_for_delivery(&mut self) -> PresentationSnapshot {
        let snapshot = self.snapshot();
        self.history_operations.clear();
        self.delivery = PresentationDelivery {
            revision: Some(self.revision),
            history_index: 0,
            pending_line_id: (!self.pending_runs.is_empty()).then_some(self.next_line),
            dirty_lines: BTreeSet::new(),
            dirty: PresentationDirty::default(),
        };
        snapshot
    }

    pub(crate) fn snapshot(&self) -> PresentationSnapshot {
        let mut lines = self.lines.clone();
        let committed_line_count = lines.len();
        if !self.pending_runs.is_empty() {
            lines.push(DisplayLine {
                line_id: self.next_line,
                temporary: self.pending_temporary,
                logical_line_start: true,
                line_end: false,
                alignment: self.current_alignment,
                runs: self.pending_runs.clone(),
            });
        }
        project_lines(
            &mut lines,
            self.project_column_cells,
            self.project_separators,
            self.settings.line_height.0,
            self.project_html,
            self.project_graphics,
        );
        // A snapshot is a self-contained replay baseline, not an ever-growing audit log.
        // Deltas retain exact edits until delivery; snapshots normalize the currently retained
        // physical rows to Append operations so resynchronization remains bounded by MaxLog.
        let history_operations = lines[..committed_line_count]
            .iter()
            .cloned()
            .map(|line| PresentationHistoryOperation::Append { line })
            .collect();
        PresentationSnapshot {
            revision: self.revision,
            title: self.title.clone(),
            history: PresentationHistory {
                logical_lines: lines,
                operations: history_operations,
            },
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
            tooltip: self.tooltip.clone(),
            resources: if self.project_graphics {
                self.resources.clone()
            } else {
                ResourceReplay::default()
            },
            html_island: self.html_island.clone(),
            redraw: RedrawState {
                enabled: self.redraw_enabled,
            },
        }
    }

    fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    fn pending_line(&self) -> Option<DisplayLine> {
        (!self.pending_runs.is_empty()).then(|| DisplayLine {
            line_id: self.next_line,
            temporary: self.pending_temporary,
            logical_line_start: true,
            line_end: false,
            alignment: self.current_alignment,
            runs: self.pending_runs.clone(),
        })
    }

    fn project_line(&self, mut line: DisplayLine) -> DisplayLine {
        project_lines(
            std::slice::from_mut(&mut line),
            self.project_column_cells,
            self.project_separators,
            self.settings.line_height.0,
            self.project_html,
            self.project_graphics,
        );
        line
    }
}

fn auto_button_groups(
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

fn auto_button_values(runs: &[DisplayRun], plain_runs: &BTreeSet<usize>) -> Vec<i64> {
    auto_button_groups(runs, plain_runs)
        .into_iter()
        .flat_map(|(_, _, text)| erabasic_html::split_auto_buttons(&text))
        .filter_map(|segment| segment.value)
        .collect()
}

fn bind_auto_buttons(
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

fn slice_text_runs(runs: &[DisplayRun], start: usize, end: usize) -> Vec<DisplayRun> {
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
            DisplayRun::HtmlDocument { document } => {
                rebind_html_nodes(&mut document.nodes, tokens);
            }
            _ => {}
        }
    }
}

fn disable_old_buttons(runs: &mut [DisplayRun], generation: u64) {
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

fn disable_old_html_buttons(nodes: &mut [erabasic_html::HtmlNode], generation: u64) {
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

fn rebind_html_nodes(
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

fn append_log_run(output: &mut String, run: &DisplayRun) {
    match run {
        DisplayRun::Text { text, .. } => output.push_str(text),
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
fn append_html_run(output: &mut String, run: &DisplayRun, line_height: LogicalLength) {
    match run {
        DisplayRun::Text { text, style, .. } => {
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

fn append_html_color(output: &mut String, color: Color) {
    output.push('#');
    let _ = write!(
        output,
        "{:02X}{:02X}{:02X}",
        color.red, color.green, color.blue
    );
}

fn append_presentation_length(
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

fn append_raw_mixed_length(output: &mut String, value: &PresentationLength) {
    match value {
        PresentationLength::Logical(LogicalLength(value)) => {
            output.push_str(&(value / 1_000).to_string());
            output.push_str("px");
        }
        PresentationLength::FontHeightHundredths(value) => output.push_str(&value.to_string()),
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
fn project_runs(
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
                        projected.push(plain_text(padding.clone(), line_height));
                    }
                    projected.extend(content);
                    if alignment == CellAlignment::Left && !padding.is_empty() {
                        projected.push(plain_text(padding, line_height));
                    }
                }
            }
            DisplayRun::Separator { pattern, .. } if !separators => {
                // A fixed 75-column projection is deterministic and independent of viewport.
                let pattern = if pattern.is_empty() { "-" } else { &pattern };
                projected.push(plain_text(
                    pattern.repeat(75).chars().take(75).collect(),
                    line_height,
                ));
            }
            DisplayRun::HtmlDocument { document } if !html => {
                projected.push(plain_text(
                    strip_markup(&erabasic_html::serialize_document(&document)),
                    line_height,
                ));
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
    projected
}

fn projected_text_width(runs: &[DisplayRun]) -> usize {
    runs.iter()
        .map(|run| match run {
            DisplayRun::Text { text, .. } => UnicodeWidthStr::width(text.as_str()),
            DisplayRun::Button { runs, .. } | DisplayRun::ColumnCell { content: runs, .. } => {
                projected_text_width(runs)
            }
            DisplayRun::HtmlDocument { document } => UnicodeWidthStr::width(
                strip_markup(&erabasic_html::serialize_document(document)).as_str(),
            ),
            DisplayRun::Image { alt_text, .. } => {
                alt_text.as_deref().map_or(0, UnicodeWidthStr::width)
            }
            DisplayRun::Shape { .. } | DisplayRun::Separator { .. } | DisplayRun::Space { .. } => 0,
        })
        .sum()
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

fn color_rgb(color: Color) -> i64 {
    (i64::from(color.red) << 16) | (i64::from(color.green) << 8) | i64::from(color.blue)
}

fn plain_text(text: String, _line_height: i64) -> DisplayRun {
    DisplayRun::Text {
        text,
        style: default_style(),
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
        font_family: Some("ＭＳ ゴシック".into()),
        font_millipoints: 18_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_delta(snapshot: &mut PresentationSnapshot, delta: PresentationDelta) {
        assert_eq!(snapshot.revision, delta.base_revision);
        for operation in delta.operations {
            match operation {
                PresentationOperation::AppendLine { line } => {
                    snapshot.history.logical_lines.push(line);
                }
                PresentationOperation::DeleteLines { count } => {
                    let retained = snapshot
                        .history
                        .logical_lines
                        .len()
                        .saturating_sub(count as usize);
                    snapshot.history.logical_lines.truncate(retained);
                }
                PresentationOperation::Clear => snapshot.history.logical_lines.clear(),
                PresentationOperation::TrimLines { count } => {
                    let count = (count as usize).min(snapshot.history.logical_lines.len());
                    snapshot.history.logical_lines.drain(..count);
                }
                PresentationOperation::SetTitle { title } => snapshot.title = title,
                PresentationOperation::SetBackgrounds { backgrounds } => {
                    snapshot.backgrounds = backgrounds;
                }
                PresentationOperation::SetAudio { audio } => snapshot.audio = audio,
                PresentationOperation::SetInputWait { input_wait } => {
                    snapshot.input_wait = input_wait;
                }
                PresentationOperation::ReplaceLine { line_id, line } => {
                    let target = snapshot
                        .history
                        .logical_lines
                        .iter_mut()
                        .find(|current| current.line_id == line_id)
                        .expect("delta replaces an existing logical line");
                    *target = line;
                }
                PresentationOperation::SetSettings { settings } => snapshot.settings = settings,
                PresentationOperation::SetTooltip { tooltip } => snapshot.tooltip = tooltip,
                PresentationOperation::SetResources { resources } => {
                    snapshot.resources = resources;
                }
                PresentationOperation::SetHtmlIsland { html_island } => {
                    snapshot.html_island = html_island;
                }
                PresentationOperation::SetRedraw { redraw } => snapshot.redraw = redraw,
                PresentationOperation::SetButtonGeneration { generation } => {
                    for line in &mut snapshot.history.logical_lines {
                        disable_old_buttons(&mut line.runs, generation);
                    }
                }
            }
        }
        snapshot.revision = delta.new_revision;
    }

    fn assert_visible_snapshot_eq(left: &PresentationSnapshot, right: &PresentationSnapshot) {
        assert_eq!(left.revision, right.revision);
        assert_eq!(left.title, right.title);
        assert_eq!(left.history.logical_lines, right.history.logical_lines);
        assert_eq!(left.backgrounds, right.backgrounds);
        assert_eq!(left.audio, right.audio);
        assert_eq!(left.input_wait, right.input_wait);
        assert_eq!(left.settings, right.settings);
        assert_eq!(left.tooltip, right.tooltip);
        assert_eq!(left.resources, right.resources);
        assert_eq!(left.html_island, right.html_island);
        assert_eq!(left.redraw, right.redraw);
    }

    #[test]
    fn presentation_deltas_replay_to_the_same_visible_state_as_a_snapshot() {
        let mut model = PresentationModel::default();
        model.set_projection(true, true, true, true, true);
        let PresentationUpdate::Snapshot(mut frontend) = model.next_update() else {
            panic!("first delivery must establish a snapshot baseline");
        };

        model.append_print_text("left".into(), false, false);
        let PresentationUpdate::Delta(delta) = model.next_update() else {
            panic!("pending text should use a delta after synchronization");
        };
        assert!(matches!(
            delta.operations.as_slice(),
            [PresentationOperation::AppendLine { line }] if !line.line_end
        ));
        apply_delta(&mut frontend, delta);
        assert_visible_snapshot_eq(&frontend, &model.snapshot());

        model.append_print_text(" right".into(), false, true);
        model.set_title("delta title".into());
        model.set_background(0x0011_2233);
        model.set_redraw(false);
        model.set_tooltip_delay(250).unwrap();
        model.set_resource_replay(ResourceReplay::default());
        model.add_background("background".into(), 2, 128);
        model.set_audio("sound".into(), false, true);
        model.append_html_island(erabasic_html::parse_document("<b>top</b>").unwrap());
        model.set_button_generation(1);
        let PresentationUpdate::Delta(delta) = model.next_update() else {
            panic!("recoverable presentation fields should remain incremental");
        };
        apply_delta(&mut frontend, delta);
        assert_visible_snapshot_eq(&frontend, &model.snapshot());

        let restored = serde_json::to_vec(&model).unwrap();
        let mut restored: PresentationModel = serde_json::from_slice(&restored).unwrap();
        assert!(matches!(
            restored.next_update(),
            PresentationUpdate::Snapshot(_)
        ));
    }

    #[test]
    fn consecutive_column_cells_share_the_pending_logical_line() {
        let mut model = PresentationModel::default();
        model.append_column_cell("A".into(), CellAlignment::Right);
        model.append_column_cell("B".into(), CellAlignment::Left);
        let pending = model.snapshot();
        assert_eq!(pending.history.logical_lines.len(), 1);
        assert!(!pending.history.logical_lines[0].line_end);
        assert_eq!(pending.history.logical_lines[0].runs.len(), 2);

        model.append_print_text("done".into(), false, true);
        let committed = model.snapshot();
        assert_eq!(committed.history.logical_lines.len(), 1);
        assert!(committed.history.logical_lines[0].line_end);
        assert_eq!(committed.history.logical_lines[0].runs.len(), 3);
    }

    #[test]
    fn plain_projection_pads_column_cells_to_their_preferred_width() {
        let mut model = PresentationModel::default();
        model.set_projection(false, false, false, false, false);
        model.append_column_cell("A".into(), CellAlignment::Right);
        model.append_column_cell("B".into(), CellAlignment::Right);
        let snapshot = model.snapshot();
        let text = snapshot.history.logical_lines[0]
            .runs
            .iter()
            .filter_map(|run| match run {
                DisplayRun::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        assert_eq!(text, format!("{}A{}B", " ".repeat(24), " ".repeat(24)));

        model.append_print_text(String::new(), false, true);
        let committed = model.snapshot();
        let Some(PresentationHistoryOperation::Append { line }) =
            committed.history.operations.last()
        else {
            panic!("committed cell line must append to physical history");
        };
        assert!(
            line.runs
                .iter()
                .all(|run| !matches!(run, DisplayRun::ColumnCell { .. }))
        );
    }

    #[test]
    fn separator_flushes_existing_text_to_an_independent_line() {
        let mut model = PresentationModel::default();
        model.append_print_text("prefix".into(), false, false);
        model.append_separator("=".into());
        let snapshot = model.snapshot();
        assert_eq!(snapshot.history.logical_lines.len(), 2);
        assert!(matches!(
            &snapshot.history.logical_lines[1].runs[0],
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
        assert_eq!(snapshot.history.logical_lines.len(), 2);
        assert!(snapshot.history.logical_lines[1].temporary);
        assert!(matches!(
            &snapshot.history.logical_lines[1].runs[0],
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
        model.append_html(erabasic_html::parse_document("<b>fallback</b>").unwrap());
        model.append_image("image.png".into(), Some("image".into()));
        model.set_audio("sound.ogg".into(), false, true);

        let fallback = model.snapshot();
        assert!(fallback.audio.is_empty());
        assert_eq!(
            fallback.history.logical_lines[0].alignment,
            LineAlignment::Center
        );
        let DisplayRun::Text { style, .. } = &fallback.history.logical_lines[0].runs[0] else {
            panic!("first run must be text");
        };
        assert!(style.bold);
        assert!(style.underline);
        assert!(matches!(
            &fallback.history.logical_lines[1].runs[0],
            DisplayRun::Text { text, .. } if text == "fallback"
        ));

        model.set_projection(true, true, true, true, true);
        let rich = model.snapshot();
        assert_eq!(rich.audio.len(), 1);
        assert!(matches!(
            rich.history.logical_lines[1].runs[0],
            DisplayRun::HtmlDocument { .. }
        ));
        assert!(matches!(
            rich.history.logical_lines[2].runs[0],
            DisplayRun::Image { .. }
        ));
    }

    #[test]
    fn style_queries_and_html_island_remain_canonical() {
        let mut model = PresentationModel::default();
        model.set_bold(true);
        model.set_italic(true);
        model.set_alignment(LineAlignment::Right);
        model.append_html_island(erabasic_html::parse_document("<b>x</b>").unwrap());
        assert_eq!(model.style_bits(), 3);
        assert_eq!(model.alignment(), LineAlignment::Right);
        assert_eq!(
            model.snapshot().html_island,
            vec![erabasic_html::parse_document("<b>x</b>").unwrap()]
        );
        model.clear_html_island();
        assert!(model.snapshot().html_island.is_empty());
    }

    #[test]
    fn plain_buttons_remain_on_the_current_logical_line() {
        let mut model = PresentationModel::default();
        model.append_button(
            "one".into(),
            ProtocolValue::Integer(1),
            InteractionToken { epoch: 1, id: 1 },
            None,
        );
        model.append_button(
            "two".into(),
            ProtocolValue::Integer(2),
            InteractionToken { epoch: 1, id: 2 },
            None,
        );
        let pending = model.snapshot();
        assert_eq!(pending.history.logical_lines.len(), 1);
        assert!(!pending.history.logical_lines[0].line_end);
        assert_eq!(pending.history.logical_lines[0].runs.len(), 2);
    }

    #[test]
    fn automatic_buttons_are_grouped_after_the_complete_print_buffer_is_committed() {
        let mut model = PresentationModel::default();
        model.append_print_text("[1] one  ".into(), false, false);
        model.set_bold(true);
        model.append_print_text("[2] two".into(), false, true);
        assert_eq!(model.last_line_auto_button_values(), vec![1, 2]);
        let tokens = [
            InteractionToken { epoch: 4, id: 1 },
            InteractionToken { epoch: 4, id: 2 },
        ];
        assert_eq!(
            model.bind_last_line_auto_buttons(&tokens),
            vec![(tokens[0], 1), (tokens[1], 2)]
        );
        assert!(
            model.snapshot().history.logical_lines[0]
                .runs
                .iter()
                .all(|run| matches!(run, DisplayRun::Button { .. }))
        );

        let mut mixed = PresentationModel::default();
        mixed.append_print_text("[1] automatic ".into(), false, false);
        mixed.append_plain_print_text("[2] plain ".into(), false, false);
        mixed.append_print_text("[3] automatic".into(), false, true);
        let tokens = [
            InteractionToken { epoch: 1, id: 3 },
            InteractionToken { epoch: 1, id: 4 },
        ];
        assert_eq!(mixed.last_line_auto_button_values(), vec![1, 3]);
        assert_eq!(
            mixed.bind_last_line_auto_buttons(&tokens),
            vec![(tokens[0], 1), (tokens[1], 3)]
        );
        assert!(matches!(
            &mixed.snapshot().history.logical_lines[0].runs[1],
            DisplayRun::Text { text, .. } if text == "[2] plain "
        ));
    }

    #[test]
    fn html_pop_serializes_semantic_button_values_and_consumes_pending_runs() {
        let mut model = PresentationModel::default();
        model.append_print_text("A<&".into(), false, false);
        model.append_button(
            "choose".into(),
            ProtocolValue::Integer(42),
            InteractionToken { epoch: 1, id: 9 },
            None,
        );
        assert_eq!(
            model.pop_printing_html(),
            "A&lt;&amp;<button value='42'>choose</button>"
        );
        assert_eq!(model.pop_printing_html(), "");
        assert!(model.last_line_is_empty());
    }

    #[test]
    fn backgrounds_and_tooltips_are_recoverable_canonical_state() {
        let mut model = PresentationModel::default();
        model.set_projection(true, true, true, true, true);
        model.add_background("BACK".into(), 2, 128);
        model.set_tooltip_colors(0x0011_2233, 0x0044_5566);
        model.set_tooltip_delay(250).unwrap();
        model.set_tooltip_duration(100_000).unwrap();
        let snapshot = model.snapshot();
        assert_eq!(snapshot.backgrounds.len(), 1);
        assert_eq!(snapshot.backgrounds[0].depth, 2);
        assert_eq!(snapshot.backgrounds[0].opacity.numerator, 128);
        assert_eq!(snapshot.backgrounds[0].opacity.denominator, 255);
        assert_eq!(snapshot.tooltip.delay_ms, 250);
        assert_eq!(snapshot.tooltip.duration_ms, i16::MAX as u32);
    }

    #[test]
    fn history_buttons_redraw_and_duplicate_backgrounds_remain_semantic() {
        let mut model = PresentationModel::default();
        model.set_projection(true, true, true, true, true);
        model.append_button(
            "old".into(),
            ProtocolValue::Integer(1),
            InteractionToken { epoch: 1, id: 1 },
            None,
        );
        model.append_text(String::new(), false);
        model.set_button_generation(1);
        model.add_background("SAME".into(), 1, 1);
        model.add_background("SAME".into(), 3, 2);
        model.set_tooltip_format(2 | 16 | (1_i64 << 40));
        model.set_redraw(false);
        let snapshot = model.snapshot();
        assert_eq!(snapshot.backgrounds.len(), 2);
        assert_eq!(snapshot.backgrounds[0].depth, 3);
        assert!(!snapshot.redraw.enabled);
        assert_eq!(
            snapshot.tooltip.normalized_format.flags,
            vec![
                era_runtime_protocol::TooltipFormatFlag::Right,
                era_runtime_protocol::TooltipFormatFlag::WordBreak,
            ]
        );
        assert_eq!(snapshot.tooltip.normalized_format.unknown_bits, 1_u64 << 40);
        assert!(
            snapshot
                .history
                .operations
                .iter()
                .all(|operation| matches!(operation, PresentationHistoryOperation::Append { .. }))
        );
        assert!(matches!(
            &snapshot.history.logical_lines[0].runs[0],
            DisplayRun::Button {
                enabled: false,
                generation: 0,
                ..
            }
        ));
        assert!(model.remove_background("SAME"));
        assert_eq!(model.snapshot().backgrounds.len(), 1);
    }

    #[test]
    fn logical_line_count_tracks_reference_clearline_semantics() {
        let mut model = PresentationModel::default();
        model.append_text("one".into(), false);
        model.append_text("two".into(), false);
        assert_eq!(model.logical_line_count(), 2);

        model.append_print_text("pending".into(), false, false);
        model.delete_last_lines(2);
        assert_eq!(model.logical_line_count(), 1);
        assert_eq!(model.snapshot().history.logical_lines.len(), 1);

        model.delete_last_lines(3);
        assert_eq!(model.logical_line_count(), -2);
    }

    #[test]
    fn max_log_trims_oldest_physical_lines_without_changing_linecount() {
        let mut model = PresentationModel::default();
        model.settings.maximum_physical_lines = 2;
        model.append_text("one".into(), false);
        model.append_text("two".into(), false);
        model.append_text("three".into(), false);

        let snapshot = model.snapshot();
        assert_eq!(model.logical_line_count(), 3);
        assert_eq!(snapshot.history.logical_lines.len(), 2);
        assert_eq!(snapshot.history.operations.len(), 2);
        assert!(
            snapshot
                .history
                .operations
                .iter()
                .all(|operation| matches!(operation, PresentationHistoryOperation::Append { .. }))
        );
    }
}
