use era_runtime_protocol::{
    CellAlignment, DisplayLine, DisplayRun, InteractionToken, LineAlignment, LogicalLength,
    MediaPlacement, PresentationHistoryOperation, PresentationLength, ProtocolValue,
    RationalOpacity, SeparatorRole, Shape, SystemTextArgument, SystemTextKey, SystemTextRef,
    TextStyle,
};
use erabasic_vm::{CharacterWidthMode, VmValue};
use std::collections::{BTreeMap, BTreeSet};

mod model;

use self::model::{PresentationDelivery, PresentationDirty};
pub(crate) use self::model::{PresentationModel, PresentationUpdate};

enum HtmlLineEvent {
    Node(Box<erabasic_html::HtmlNode>),
    Break,
}

fn split_html_breaks(node: erabasic_html::HtmlNode, output: &mut Vec<HtmlLineEvent>) {
    let erabasic_html::HtmlNode::Element {
        kind,
        attributes,
        children,
        interaction,
        start,
        end,
        semantic,
    } = node
    else {
        output.push(HtmlLineEvent::Node(Box::new(node)));
        return;
    };
    if kind == erabasic_html::HtmlElementKind::Break {
        output.push(HtmlLineEvent::Break);
        return;
    }
    if kind == erabasic_html::HtmlElementKind::Division || children.is_empty() {
        output.push(HtmlLineEvent::Node(Box::new(
            erabasic_html::HtmlNode::Element {
                kind,
                attributes,
                children,
                interaction,
                start,
                end,
                semantic,
            },
        )));
        return;
    }
    let mut events = Vec::new();
    for child in children {
        split_html_breaks(child, &mut events);
    }
    let mut line = Vec::new();
    for event in events {
        match event {
            HtmlLineEvent::Node(child) => line.push(*child),
            HtmlLineEvent::Break => {
                if !line.is_empty() {
                    output.push(HtmlLineEvent::Node(Box::new(
                        erabasic_html::HtmlNode::Element {
                            kind,
                            attributes: attributes.clone(),
                            children: std::mem::take(&mut line),
                            interaction: interaction.clone(),
                            start,
                            end,
                            semantic: semantic.clone(),
                        },
                    )));
                }
                output.push(HtmlLineEvent::Break);
            }
        }
    }
    if !line.is_empty() {
        output.push(HtmlLineEvent::Node(Box::new(
            erabasic_html::HtmlNode::Element {
                kind,
                attributes,
                children: line,
                interaction,
                start,
                end,
                semantic,
            },
        )));
    }
}

impl Default for PresentationModel {
    fn default() -> Self {
        defaults::model()
    }
}

impl PresentationModel {
    pub(crate) fn set_character_width_mode(&mut self, mode: CharacterWidthMode) {
        if self.character_width_mode != mode {
            self.character_width_mode = mode;
            self.delivery.dirty.force_snapshot = true;
        }
    }

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

    pub(crate) fn enabled_button_value(&self, token: InteractionToken) -> Option<VmValue> {
        self.pending_runs
            .iter()
            .rev()
            .find_map(|run| enabled_button_value(run, token))
            .or_else(|| {
                self.lines.iter().rev().find_map(|line| {
                    line.runs
                        .iter()
                        .rev()
                        .find_map(|run| enabled_button_value(run, token))
                })
            })
    }

    pub(crate) fn replay_button(
        &self,
        token: InteractionToken,
        value: crate::input_replay::ReplayValue,
    ) -> Option<crate::input_replay::ReplayButton> {
        let mut candidates = Vec::new();
        for line in &self.lines {
            projection::collect_replay_buttons(&line.runs, &mut candidates);
        }
        projection::collect_replay_buttons(&self.pending_runs, &mut candidates);
        let (index, candidate) = candidates
            .into_iter()
            .enumerate()
            .find(|(_, candidate)| candidate.token == token)?;
        Some(crate::input_replay::ReplayButton {
            visible_text: candidate.visible_text,
            title: candidate.title,
            alt_text: candidate.alt_text,
            value,
            ordinal: index.saturating_add(1),
        })
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
        let mut physical_count = 0usize;
        if count != 0 && !self.pending_runs.is_empty() {
            self.pending_runs.clear();
            self.pending_temporary = false;
            count -= 1;
            physical_count += 1;
        }
        let mut logical_deletions = 0usize;
        while count != 0 {
            let Some(line) = self.lines.pop() else { break };
            physical_count += 1;
            if line.logical_line_start {
                count -= 1;
                logical_deletions += 1;
            }
        }
        let logical_deletions = i64::try_from(logical_deletions).unwrap_or(i64::MAX);
        self.logical_line_count = self.logical_line_count.wrapping_sub(logical_deletions);
        self.line_count_dirty = true;
        self.history_operations
            .push(PresentationHistoryOperation::DeletePhysical {
                count: u32::try_from(physical_count).unwrap_or(u32::MAX),
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
            // The configured PrintCLength is layout intent, not padding.
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
        let mut style = self.current_style.clone();
        // Emuera renders DRAWLINE with the active colors and font face/size, but
        // deliberately resets the four FontStyle flags to Regular for the rule.
        style.bold = false;
        style.italic = false;
        style.underline = false;
        style.strikeout = false;
        self.pending_runs.push(DisplayRun::Separator {
            pattern,
            role: SeparatorRole::Rule,
            style,
        });
        self.bump();
        self.commit_line();
    }

    pub(crate) fn append_html(&mut self, document: erabasic_html::HtmlDocument) {
        if !self.pending_runs.is_empty() {
            self.commit_line();
        }
        let mut current = Vec::new();
        let mut events = Vec::new();
        for node in document.nodes {
            split_html_breaks(node, &mut events);
        }
        let mut logical_start = true;
        for event in events {
            let node = match event {
                HtmlLineEvent::Node(node) => *node,
                HtmlLineEvent::Break => {
                    if !current.is_empty() {
                        self.pending_runs.push(DisplayRun::HtmlDocument {
                            document: erabasic_html::HtmlDocument {
                                nodes: std::mem::take(&mut current),
                            },
                        });
                    }
                    self.commit_line_with_logical_start(logical_start);
                    logical_start = false;
                    continue;
                }
            };
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
                    self.commit_line_with_logical_start(logical_start);
                    logical_start = false;
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
                        self.commit_line_with_logical_start(logical_start);
                        logical_start = false;
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
                    self.commit_line_with_logical_start(logical_start);
                    logical_start = false;
                    self.current_alignment = previous;
                }
                _ => current.push(node),
            }
        }
        if !current.is_empty() {
            self.pending_runs.push(DisplayRun::HtmlDocument {
                document: erabasic_html::HtmlDocument { nodes: current },
            });
            self.commit_line_with_logical_start(logical_start);
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
        self.current_style = self.default_style.clone();
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

    fn apply_project_default_style(&mut self, next: TextStyle) {
        let previous = std::mem::replace(&mut self.default_style, next.clone());
        for line in &mut self.lines {
            if replace_project_default_style(&mut line.runs, &previous, &next) {
                self.delivery.dirty_lines.insert(line.line_id);
            }
        }
        replace_project_default_style(&mut self.pending_runs, &previous, &next);
        self.current_style = next;
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

    fn commit_line(&mut self) {
        self.commit_line_with_logical_start(true);
    }

    fn commit_line_with_logical_start(&mut self, logical_line_start: bool) {
        self.last_committed_plain_runs = std::mem::take(&mut self.pending_plain_runs);
        let line = DisplayLine {
            line_id: self.next_line,
            temporary: self.pending_temporary,
            logical_line_start,
            line_end: true,
            alignment: self.current_alignment,
            runs: std::mem::take(&mut self.pending_runs),
        };
        self.pending_temporary = false;
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push(line.clone());
        if logical_line_start {
            self.logical_line_count = if self.logical_line_count == i64::MAX {
                0
            } else {
                self.logical_line_count + 1
            };
        }
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
}

fn replace_project_default_style(
    runs: &mut [DisplayRun],
    previous: &TextStyle,
    next: &TextStyle,
) -> bool {
    let mut changed = false;
    for run in runs {
        match run {
            DisplayRun::Text { style, .. }
            | DisplayRun::TextLayout { style, .. }
            | DisplayRun::Separator { style, .. } => {
                changed |= replace_matching_style_defaults(style, previous, next);
            }
            DisplayRun::Button {
                runs, hover_style, ..
            } => {
                changed |= replace_project_default_style(runs, previous, next);
                if let Some(style) = hover_style {
                    changed |= replace_matching_style_defaults(style, previous, next);
                }
            }
            DisplayRun::ColumnCell { content, .. } => {
                changed |= replace_project_default_style(content, previous, next);
            }
            DisplayRun::HtmlDocument { .. }
            | DisplayRun::Image { .. }
            | DisplayRun::Shape { .. }
            | DisplayRun::Space { .. } => {}
        }
    }
    changed
}

fn replace_matching_style_defaults(
    style: &mut TextStyle,
    previous: &TextStyle,
    next: &TextStyle,
) -> bool {
    let mut changed = false;
    if style.font_family == previous.font_family && style.font_family != next.font_family {
        style.font_family.clone_from(&next.font_family);
        changed = true;
    }
    if style.font_millipixels == previous.font_millipixels
        && style.font_millipixels != next.font_millipixels
    {
        style.font_millipixels = next.font_millipixels;
        changed = true;
    }
    if style.foreground == previous.foreground && style.foreground != next.foreground {
        style.foreground = next.foreground;
        changed = true;
    }
    changed
}

mod defaults;
mod delivery;
mod media;
mod projection;
#[cfg(test)]
mod tests;

pub(crate) use self::projection::display_value;
use self::projection::{
    append_html_run, append_log_run, auto_button_values, bind_auto_buttons, color_rgb,
    disable_old_buttons, enabled_button_value, project_lines, rebind_runs, rgb_color, run_is_empty,
};
