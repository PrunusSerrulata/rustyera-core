use super::projection::{append_plain_run, append_printed_html_run};
use super::{
    PresentationDelivery, PresentationDirty, PresentationHistoryEdit, PresentationModel,
    PresentationUpdate, project_lines,
};
use era_runtime_protocol::{
    DisplayLine, InputWait, LineAlignment, PresentationDelta, PresentationHistory,
    PresentationHistoryOperation, PresentationOperation, PresentationSnapshot, RedrawState,
    ResourceReplay,
};
use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

impl PresentationModel {
    pub(crate) fn display_line(&self, raw_index: i64, from_end: bool) -> String {
        let index = if from_end && raw_index < 0 {
            let distance = usize::try_from(raw_index.unsigned_abs()).unwrap_or(usize::MAX);
            self.lines.len().checked_sub(distance)
        } else {
            usize::try_from(raw_index).ok()
        };
        let Some(line) = index.and_then(|index| self.projected_line(index)) else {
            return String::new();
        };
        let mut output = String::new();
        for run in &line.runs {
            append_plain_run(&mut output, run);
        }
        output
    }

    pub(crate) fn printed_html_line(&self, index_from_end: usize) -> String {
        let mut logical_index = 0_usize;
        let mut selected = Vec::new();
        for physical_index in (0..self.delivered_line_count()).rev() {
            let Some(line) = self.projected_line(physical_index) else {
                continue;
            };
            if logical_index == index_from_end {
                selected.push(line.clone());
            }
            if line.logical_line_start {
                logical_index = logical_index.saturating_add(1);
            }
            if logical_index > index_from_end {
                break;
            }
        }
        selected.reverse();
        let Some(first_line) = selected.first() else {
            return String::new();
        };
        let alignment = match first_line.alignment {
            LineAlignment::Left => "left",
            LineAlignment::Center => "center",
            LineAlignment::Right => "right",
        };
        let mut output = format!("<p align='{alignment}'><nobr>");
        for (index, line) in selected.iter().enumerate() {
            if index != 0 {
                output.push_str("<br>");
            }
            for run in &line.runs {
                append_printed_html_run(&mut output, run, self.settings.line_height);
            }
        }
        output.push_str("</nobr></p>");
        output
    }

    pub(crate) fn has_wait(&self, wait_id: u64) -> bool {
        self.input_wait
            .as_ref()
            .is_some_and(|wait| wait.wait_id == wait_id)
    }

    pub(crate) fn has_open_wait(&self) -> bool {
        self.input_wait.is_some()
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
            || self.delivery.history_index > self.history_edits.len()
            || self.delivery.dirty.force_snapshot
        {
            return PresentationUpdate::Snapshot(Box::new(self.snapshot_for_delivery()));
        }

        let mut delivery = std::mem::take(&mut self.delivery);
        let base_revision = delivery
            .revision
            .expect("the snapshot guard establishes a delivery revision");
        let history = compact_history_edits(
            &self.history_edits[delivery.history_index..],
            delivery.history_line_count,
            delivery.pending_line_id,
        );
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
        if delivery.dirty.scene {
            operations.push(PresentationOperation::ApplySceneDelta {
                delta: self.projected_scene_delta(delivery.scene_revision),
            });
            delivery.scene_revision = self.scene.revision;
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
                resources: self.projected_resources(),
            });
        }
        if delivery.dirty.html_island {
            operations.push(PresentationOperation::SetHtmlIsland {
                html_island: self.project_html_island(),
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
                PresentationHistoryEdit::Append { line } => {
                    let line_id = line.line_id;
                    let line = if delivery.dirty_lines.remove(&line_id) {
                        self.lines
                            .iter()
                            .rev()
                            .find(|current| current.line_id == line_id)
                            .map_or_else(
                                || line.as_ref().clone(),
                                |current| current.as_ref().clone(),
                            )
                    } else {
                        line.as_ref().clone()
                    };
                    let line = self.project_line(line);
                    if delivery.pending_line_id == Some(line_id) {
                        operations.push(PresentationOperation::ReplaceLine { line_id, line });
                        delivery.pending_line_id = None;
                    } else {
                        operations.push(PresentationOperation::AppendLine { line });
                    }
                }
                PresentationHistoryEdit::ReplaceTemporary { line } => {
                    let line_id = line.line_id;
                    let line = if delivery.dirty_lines.remove(&line_id) {
                        self.lines
                            .iter()
                            .rev()
                            .find(|current| current.line_id == line_id)
                            .map_or_else(
                                || line.as_ref().clone(),
                                |current| current.as_ref().clone(),
                            )
                    } else {
                        line.as_ref().clone()
                    };
                    operations.push(PresentationOperation::DeleteLines { count: 1 });
                    operations.push(PresentationOperation::AppendLine {
                        line: self.project_line(line),
                    });
                    delivery.pending_line_id = None;
                }
                PresentationHistoryEdit::DeletePhysical { count } => {
                    operations.push(PresentationOperation::DeleteLines { count });
                    delivery.pending_line_id = None;
                }
                PresentationHistoryEdit::SetButtonGeneration { generation } => {
                    operations.push(PresentationOperation::SetButtonGeneration { generation });
                }
                PresentationHistoryEdit::TrimPhysical { count } => {
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
                .map(|current| current.as_ref().clone())
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
        delivery.history_line_count = self.delivered_line_count();
        // Encoded outbound envelopes own retransmission after this point. Keeping the
        // same semantic edits in the presentation model made long sessions grow forever.
        self.history_operations.clear();
        self.history_edits.clear();
        self.scene_operations.clear();
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
        self.history_edits.clear();
        self.scene_operations.clear();
        self.delivery = PresentationDelivery {
            revision: Some(self.revision),
            history_index: 0,
            history_line_count: self.delivered_line_count(),
            pending_line_id: (!self.pending_runs.is_empty()).then_some(self.next_line),
            scene_revision: self.scene.revision,
            dirty_lines: BTreeSet::new(),
            dirty: PresentationDirty::default(),
        };
        snapshot
    }

    pub(crate) fn snapshot(&self) -> PresentationSnapshot {
        let mut lines = self
            .lines
            .iter()
            .map(|line| line.as_ref().clone())
            .collect::<Vec<_>>();
        let committed_line_count = lines.len();
        if !self.pending_runs.is_empty() {
            lines.push(DisplayLine {
                line_id: self.next_line,
                temporary: self.pending_temporary,
                logical_line_start: true,
                line_end: false,
                alignment: self.current_alignment,
                runs: self.pending_runs.clone(),
                text_background_eligible: super::line_has_text_background(&self.pending_runs),
            });
        }
        for line in &mut lines {
            super::projection::disable_old_buttons(&mut line.runs, self.button_generation);
        }
        project_lines(
            &mut lines,
            self.project_column_cells,
            self.project_separators,
            self.settings.line_height.0,
            self.project_html,
            self.project_graphics,
            self.character_width_mode,
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
            scene: self.projected_scene(),
            audio: if self.project_audio {
                self.audio.clone()
            } else {
                Vec::new()
            },
            input_wait: self.input_wait.clone(),
            settings: self.settings.clone(),
            tooltip: self.tooltip.clone(),
            resources: self.projected_resources(),
            html_island: self.project_html_island(),
            redraw: RedrawState {
                enabled: self.redraw_enabled,
            },
        }
    }

    pub(super) fn bump(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    fn projected_resources(&self) -> ResourceReplay {
        if self.project_graphics {
            return self.resources.clone();
        }
        ResourceReplay {
            animation_timer_ms: self.resources.animation_timer_ms,
            ..ResourceReplay::default()
        }
    }

    fn pending_line(&self) -> Option<DisplayLine> {
        (!self.pending_runs.is_empty()).then(|| DisplayLine {
            line_id: self.next_line,
            temporary: self.pending_temporary,
            logical_line_start: true,
            line_end: false,
            alignment: self.current_alignment,
            runs: self.pending_runs.clone(),
            text_background_eligible: super::line_has_text_background(&self.pending_runs),
        })
    }

    fn projected_line(&self, index: usize) -> Option<DisplayLine> {
        let line = self
            .lines
            .get(index)
            .map(|line| line.as_ref().clone())
            .or_else(|| {
                (index == self.lines.len())
                    .then(|| self.pending_line())
                    .flatten()
            })?;
        Some(self.project_line(line))
    }

    fn delivered_line_count(&self) -> usize {
        self.lines.len() + usize::from(!self.pending_runs.is_empty())
    }

    fn project_line(&self, mut line: DisplayLine) -> DisplayLine {
        super::projection::disable_old_buttons(&mut line.runs, self.button_generation);
        project_lines(
            std::slice::from_mut(&mut line),
            self.project_column_cells,
            self.project_separators,
            self.settings.line_height.0,
            self.project_html,
            self.project_graphics,
            self.character_width_mode,
        );
        line
    }

    fn project_html_island(&self) -> Vec<erabasic_html::HtmlDocument> {
        let mut documents = self.html_island.clone();
        for document in &mut documents {
            super::projection::disable_old_html_buttons(
                &mut document.nodes,
                self.button_generation,
            );
        }
        documents
    }
}

fn compact_history_edits(
    history: &[PresentationHistoryEdit],
    baseline_line_count: usize,
    pending_line_id: Option<u64>,
) -> Vec<PresentationHistoryEdit> {
    // An uncommitted line is represented separately by `pending_line_id`. Preserve the exact
    // prefix through the edit that releases that frontend-only state, then reduce the remaining
    // ordinary history. Disabling the reducer for the whole batch retained every intermediate
    // redraw after a prompt and made skipped animations expensive to project and transfer.
    if let Some(pending_line_id) = pending_line_id {
        let mut line_count = baseline_line_count;
        for (index, operation) in history.iter().enumerate() {
            let releases_pending = match operation {
                PresentationHistoryEdit::Append { line } => {
                    if line.line_id == pending_line_id {
                        true
                    } else {
                        line_count = line_count.saturating_add(1);
                        false
                    }
                }
                PresentationHistoryEdit::ReplaceTemporary { .. } => true,
                PresentationHistoryEdit::DeletePhysical { count }
                | PresentationHistoryEdit::TrimPhysical { count } => {
                    line_count =
                        line_count.saturating_sub(usize::try_from(*count).unwrap_or(usize::MAX));
                    true
                }
                PresentationHistoryEdit::SetButtonGeneration { .. } => false,
            };
            if releases_pending {
                let mut compacted = history[..=index].to_vec();
                compacted.extend(compact_history_edits(
                    &history[index + 1..],
                    line_count,
                    None,
                ));
                return compacted;
            }
        }
        return history.to_vec();
    }

    let mut base_remaining = baseline_line_count;
    let mut front_trim = 0_u32;
    let mut tail_delete = 0_u32;
    let mut appends = VecDeque::<Arc<DisplayLine>>::new();
    let mut generation = None;

    let delete_tail = |count: u32,
                       base_remaining: &mut usize,
                       tail_delete: &mut u32,
                       appends: &mut VecDeque<Arc<DisplayLine>>| {
        let mut remaining = usize::try_from(count).unwrap_or(usize::MAX);
        let appended = remaining.min(appends.len());
        appends.truncate(appends.len() - appended);
        remaining -= appended;
        let from_base = remaining.min(*base_remaining);
        *base_remaining -= from_base;
        *tail_delete = tail_delete.saturating_add(u32::try_from(from_base).unwrap_or(u32::MAX));
    };

    for operation in history {
        match operation {
            PresentationHistoryEdit::Append { line } => appends.push_back(Arc::clone(line)),
            PresentationHistoryEdit::ReplaceTemporary { line } => {
                delete_tail(1, &mut base_remaining, &mut tail_delete, &mut appends);
                appends.push_back(Arc::clone(line));
            }
            PresentationHistoryEdit::DeletePhysical { count } => {
                delete_tail(*count, &mut base_remaining, &mut tail_delete, &mut appends);
            }
            PresentationHistoryEdit::SetButtonGeneration { generation: next } => {
                generation = Some(*next);
            }
            PresentationHistoryEdit::TrimPhysical { count } => {
                let mut remaining = usize::try_from(*count).unwrap_or(usize::MAX);
                let from_base = remaining.min(base_remaining);
                base_remaining -= from_base;
                remaining -= from_base;
                front_trim =
                    front_trim.saturating_add(u32::try_from(from_base).unwrap_or(u32::MAX));
                let from_appends = remaining.min(appends.len());
                appends.drain(..from_appends);
            }
        }
    }

    let mut compacted = Vec::with_capacity(appends.len().saturating_add(4));
    if front_trim != 0 {
        compacted.push(PresentationHistoryEdit::TrimPhysical { count: front_trim });
    }
    if tail_delete != 0 {
        compacted.push(PresentationHistoryEdit::DeletePhysical { count: tail_delete });
    }
    if let Some(generation) = generation {
        compacted.push(PresentationHistoryEdit::SetButtonGeneration { generation });
    }
    compacted.extend(
        appends
            .into_iter()
            .map(|line| PresentationHistoryEdit::Append { line }),
    );
    compacted
}
