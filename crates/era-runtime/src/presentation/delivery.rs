use super::{
    PresentationDelivery, PresentationDirty, PresentationModel, PresentationUpdate, project_lines,
};
use era_runtime_protocol::{
    DisplayLine, InputWait, PresentationDelta, PresentationHistory, PresentationHistoryOperation,
    PresentationOperation, PresentationSnapshot, RedrawState, ResourceReplay,
};
use std::collections::BTreeSet;

impl PresentationModel {
    pub(crate) fn has_wait(&self, wait_id: u64) -> bool {
        self.input_wait
            .as_ref()
            .is_some_and(|wait| wait.wait_id == wait_id)
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
                    self.projected_backgrounds()
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
                self.projected_backgrounds()
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

    pub(super) fn bump(&mut self) {
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
