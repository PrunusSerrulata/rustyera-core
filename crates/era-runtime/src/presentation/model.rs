//! Canonical presentation state and non-serialized delivery bookkeeping.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;

use era_runtime_protocol::{
    AudioState, Color, DisplayLine, DisplayRun, InputWait, InteractionToken, LineAlignment,
    PresentationDelta, PresentationHistoryOperation, PresentationSettings, PresentationSnapshot,
    ResourceReplay, SceneOperationV1, SceneSourceV1, SceneStateV1, TextStyle, TooltipSettings,
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
    pub(super) lines: VecDeque<Arc<DisplayLine>>,
    // Retained for runtime-snapshot schema compatibility. Delivery bookkeeping is rebuilt after
    // restore, so new sessions keep non-duplicating shared edits in `history_edits` instead.
    pub(super) history_operations: Vec<PresentationHistoryOperation>,
    #[serde(skip, default)]
    pub(super) history_edits: Vec<PresentationHistoryEdit>,
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
    #[serde(default)]
    pub(super) scene: SceneStateV1,
    #[serde(skip, default)]
    pub(super) scene_operations: Vec<SceneOperationV1>,
    /// Non-visual SETBGIMAGE lookup index; `scene` remains the sole rendered authority.
    #[serde(default)]
    pub(super) background_layers: Vec<(String, u64)>,
    #[serde(default)]
    pub(super) cbg_layers: Vec<CbgLayerIndex>,
    #[serde(default)]
    pub(super) cbg_button_map: Option<SceneSourceV1>,
    #[serde(default)]
    pub(super) image_layers: Vec<ImageLayerIndex>,
    #[serde(default = "first_scene_identifier")]
    pub(super) next_scene_layer_id: u64,
    #[serde(default = "first_scene_identifier")]
    pub(super) next_scene_sequence: u64,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CbgLayerIndex {
    pub(super) layer_id: u64,
    pub(super) depth: i64,
    pub(super) interaction: Option<InteractionToken>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct ImageLayerIndex {
    pub(super) layer_id: u64,
    pub(super) depth: i64,
}

#[derive(Clone, Debug, Default)]
pub(super) struct PresentationDelivery {
    pub(super) revision: Option<u64>,
    pub(super) history_index: usize,
    /// Physical rows held by the frontend at the delivery baseline, including one pending row.
    pub(super) history_line_count: usize,
    pub(super) pending_line_id: Option<u64>,
    pub(super) scene_revision: u64,
    pub(super) dirty_lines: BTreeSet<u64>,
    pub(super) dirty: PresentationDirty,
}

#[derive(Clone, Debug)]
pub(super) enum PresentationHistoryEdit {
    Append { line: Arc<DisplayLine> },
    ReplaceTemporary { line: Arc<DisplayLine> },
    DeletePhysical { count: u32 },
    SetButtonGeneration { generation: u64 },
    TrimPhysical { count: u32 },
}

#[derive(Clone, Copy, Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub(super) struct PresentationDirty {
    pub(super) title: bool,
    pub(super) scene: bool,
    pub(super) audio: bool,
    pub(super) input_wait: bool,
    pub(super) settings: bool,
    pub(super) tooltip: bool,
    pub(super) resources: bool,
    pub(super) html_island: bool,
    pub(super) redraw: bool,
    pub(super) force_snapshot: bool,
}

const fn first_scene_identifier() -> u64 {
    1
}

pub(crate) enum PresentationUpdate {
    Snapshot(Box<PresentationSnapshot>),
    Delta(PresentationDelta),
}
