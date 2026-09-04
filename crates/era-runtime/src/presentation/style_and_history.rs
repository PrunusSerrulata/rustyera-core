impl PresentationModel {
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
            let line = Arc::make_mut(line);
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

    pub(crate) fn set_text_line_background(&mut self, color: Option<Color>) {
        self.settings.text_line_background = color;
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
        self.history_edits
            .push(PresentationHistoryEdit::SetButtonGeneration { generation });
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
        self.last_committed_plain_runs = std::mem::take(&mut self.pending_plain_runs);
        let runs = std::mem::take(&mut self.pending_runs);
        let line = Arc::new(DisplayLine {
            line_id: self.next_line,
            temporary: self.pending_temporary,
            logical_line_start: true,
            line_end: true,
            alignment: self.current_alignment,
            text_background_eligible: line_has_text_background(&runs),
            runs,
        });
        self.pending_temporary = false;
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push_back(Arc::clone(&line));
        self.logical_line_count = if self.logical_line_count == i64::MAX {
            0
        } else {
            self.logical_line_count + 1
        };
        self.advance_canonical_document_cursor();
        self.line_count_dirty = true;
        if self.replace_next_temporary {
            self.history_edits
                .push(PresentationHistoryEdit::ReplaceTemporary { line });
            self.replace_next_temporary = false;
        } else {
            self.history_edits
                .push(PresentationHistoryEdit::Append { line });
        }
        self.trim_physical_history();
        self.bump();
    }

    fn record_history_delete(&mut self, mut count: u32) {
        if self.delivery.pending_line_id.is_none() {
            while count != 0 {
                let Some(index) = self.history_edits.iter().rposition(|operation| {
                    !matches!(
                        operation,
                        PresentationHistoryEdit::SetButtonGeneration { .. }
                    )
                }) else {
                    break;
                };
                if !matches!(
                    self.history_edits[index],
                    PresentationHistoryEdit::Append { .. }
                ) {
                    break;
                }
                self.history_edits.remove(index);
                count -= 1;
            }
        }
        if count == 0 {
            return;
        }
        if let Some(PresentationHistoryEdit::DeletePhysical { count: previous }) =
            self.history_edits.last_mut()
        {
            *previous = previous.saturating_add(count);
        } else {
            self.history_edits
                .push(PresentationHistoryEdit::DeletePhysical { count });
        }
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
                width: CellWidthIntent::ProjectColumns(self.print_c_length),
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
        let runs = vec![self.button_run(
            text,
            ProtocolValue::String(String::new()),
            token,
            system_text,
        )];
        let line = Arc::new(DisplayLine {
            line_id: self.next_line,
            temporary: false,
            logical_line_start: true,
            line_end: true,
            alignment: self.current_alignment,
            text_background_eligible: line_has_text_background(&runs),
            runs,
        });
        self.next_line = self.next_line.saturating_add(1);
        self.lines.push_back(Arc::clone(&line));
        self.logical_line_count = if self.logical_line_count == i64::MAX {
            0
        } else {
            self.logical_line_count + 1
        };
        self.advance_canonical_document_cursor();
        self.line_count_dirty = true;
        self.history_edits
            .push(PresentationHistoryEdit::Append { line });
        self.trim_physical_history();
        self.bump();
    }

    fn trim_physical_history(&mut self) {
        let maximum = self.settings.maximum_physical_lines as usize;
        let excess = self.lines.len().saturating_sub(maximum);
        if excess == 0 {
            return;
        }
        let removed_line_ids = self
            .lines
            .iter()
            .take(excess)
            .map(|line| line.line_id)
            .collect::<Vec<_>>();
        self.lines.drain(..excess);
        self.clear_anchored_scene_lines(&removed_line_ids);
        let count = u32::try_from(excess).unwrap_or(u32::MAX);
        if let Some(PresentationHistoryEdit::TrimPhysical { count: previous }) =
            self.history_edits.last_mut()
        {
            *previous = previous.saturating_add(count);
        } else {
            self.history_edits
                .push(PresentationHistoryEdit::TrimPhysical { count });
        }
    }

    fn advance_canonical_document_cursor(&mut self) {
        self.canonical_document_cursor_y.0 = self
            .canonical_document_cursor_y
            .0
            .saturating_add(self.settings.line_height.0.max(0));
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
