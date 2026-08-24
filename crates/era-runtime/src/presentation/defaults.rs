use std::collections::{BTreeSet, VecDeque};

use era_runtime_protocol::{
    Color, LineAlignment, LogicalLength, PresentationSettings, ResourceReplay, TooltipFormat,
    TooltipSettings,
};
use erabasic_vm::CharacterWidthMode;

use super::{PresentationDelivery, PresentationModel};
use crate::presentation::projection::{default_style, rgb_color};

pub(super) fn model() -> PresentationModel {
    PresentationModel {
        revision: 0,
        title: String::new(),
        lines: VecDeque::new(),
        history_operations: Vec::new(),
        history_edits: Vec::new(),
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
            drawable_height: LogicalLength(480_000),
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
        client_backgrounds: Vec::new(),
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
        resource_replay_stale: false,
        print_c_length: 25,
        character_width_mode: CharacterWidthMode::Automatic,
        delivery: PresentationDelivery::default(),
    }
}
