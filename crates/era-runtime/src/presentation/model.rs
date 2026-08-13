//! Canonical presentation state and non-serialized delivery bookkeeping.

use std::collections::BTreeSet;

use era_runtime_protocol::{
    AudioState, Color, DisplayLine, DisplayRun, InputWait, LineAlignment, MediaPlacement,
    PresentationDelta, PresentationHistoryOperation, PresentationSettings, PresentationSnapshot,
    ResourceReplay, TextStyle, TooltipSettings,
};
use erabasic_vm::CharacterWidthMode;
use serde::{Deserialize, Serialize};

const fn dirty_line_count() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct PresentationModel {
    pub(super) revision: u64,
    pub(super) title: String,
    pub(super) lines: Vec<DisplayLine>,
    pub(super) history_operations: Vec<PresentationHistoryOperation>,
    pub(super) pending_runs: Vec<DisplayRun>,
    pub(super) pending_plain_runs: BTreeSet<usize>,
    pub(super) last_committed_plain_runs: BTreeSet<usize>,
    pub(super) pending_temporary: bool,
    pub(super) input_wait: Option<InputWait>,
    pub(super) next_line: u64,
    pub(super) logical_line_count: i64,
    #[serde(skip, default = "dirty_line_count")]
    pub(super) line_count_dirty: bool,
    pub(super) settings: PresentationSettings,
    pub(super) project_column_cells: bool,
    pub(super) project_separators: bool,
    pub(super) project_html: bool,
    pub(super) project_graphics: bool,
    pub(super) project_audio: bool,
    pub(super) current_style: TextStyle,
    pub(super) default_style: TextStyle,
    pub(super) default_background: Color,
    pub(super) current_alignment: LineAlignment,
    pub(super) redraw_enabled: bool,
    pub(super) button_generation: u64,
    pub(super) replace_next_temporary: bool,
    pub(super) html_island: Vec<erabasic_html::HtmlDocument>,
    pub(super) backgrounds: Vec<MediaPlacement>,
    #[serde(default)]
    pub(super) client_backgrounds: Vec<MediaPlacement>,
    pub(super) audio: Vec<AudioState>,
    pub(super) tooltip: TooltipSettings,
    pub(super) resources: ResourceReplay,
    #[serde(default)]
    pub(super) resource_replay_stale: bool,
    pub(super) print_c_length: u32,
    #[serde(skip)]
    pub(super) character_width_mode: CharacterWidthMode,
    #[serde(skip)]
    pub(super) delivery: PresentationDelivery,
}

#[derive(Clone, Debug, Default)]
pub(super) struct PresentationDelivery {
    pub(super) revision: Option<u64>,
    pub(super) history_index: usize,
    pub(super) pending_line_id: Option<u64>,
    pub(super) dirty_lines: BTreeSet<u64>,
    pub(super) dirty: PresentationDirty,
}

#[derive(Clone, Copy, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub(super) struct PresentationDirty {
    pub(super) title: bool,
    pub(super) backgrounds: bool,
    pub(super) audio: bool,
    pub(super) input_wait: bool,
    pub(super) settings: bool,
    pub(super) tooltip: bool,
    pub(super) resources: bool,
    pub(super) html_island: bool,
    pub(super) redraw: bool,
    pub(super) force_snapshot: bool,
}

pub(crate) enum PresentationUpdate {
    Snapshot(Box<PresentationSnapshot>),
    Delta(PresentationDelta),
}
